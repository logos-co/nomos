pub mod backends;
pub mod network;
pub mod settings;

use std::{
    fmt::{Debug, Display},
    future::Future,
    hash::Hash,
    time::Duration,
};

use async_trait::async_trait;
use backends::BlendBackend;
use futures::{future::join_all, StreamExt as _};
use network::NetworkAdapter;
use nomos_blend_message::{crypto::random_sized_bytes, encap::DecapsulationOutput, PayloadType};
use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::crypto::CryptographicProcessor,
    message_scheduler::{round_info::RoundInfo, MessageScheduler},
    UninitializedMessageScheduler,
};
use nomos_core::wire;
use nomos_network::NetworkService;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{seq::SliceRandom as _, RngCore, SeedableRng as _};
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use crate::{
    core::settings::BlendConfig,
    message::{NetworkMessage, ProcessedMessage, ServiceMessage},
};

const LOG_TARGET: &str = "blend::service::core";

/// A blend service that sends messages to the blend network
/// and broadcasts fully unwrapped messages through the [`NetworkService`].
///
/// The blend backend and the network adapter are generic types that are
/// independent of each other. For example, the blend backend can use the
/// libp2p network stack, while the network adapter can use the other network
/// backend.
pub struct BlendService<Backend, NodeId, Network, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: Membership<NodeId>,
}

impl<Backend, NodeId, Network, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, Network, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    type Settings = BlendConfig<Backend::Settings, NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<Network::BroadcastSettings>;
}

#[async_trait]
impl<Backend, NodeId, Network, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, Network, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync,
    RuntimeServiceId: AsServiceId<NetworkService<Network::Backend, RuntimeServiceId>>
        + AsServiceId<Self>
        + Clone
        + Debug
        + Display
        + Sync
        + Send
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let settings_reader = service_resources_handle.settings_handle.notifier();
        let blend_config = settings_reader.get_updated_settings();
        let membership = blend_config.membership();
        Ok(Self {
            backend: <Backend as BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId>>::new(
                settings_reader.get_updated_settings(),
                service_resources_handle.overwatch_handle.clone(),
                Box::pin(
                    IntervalStream::new(interval(blend_config.time.session_duration()))
                        .map(move |_| membership.clone()),
                ),
                ChaCha12Rng::from_entropy(),
            ),
            service_resources_handle,
            membership: blend_config.membership(),
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    ref overwatch_handle,
                    ref settings_handle,
                    ref status_updater,
                    ..
                },
            ref mut backend,
            ref membership,
        } = self;
        let blend_config = settings_handle.notifier().get_updated_settings();
        let mut rng = ChaCha12Rng::from_entropy();
        let mut cryptographic_processor = CryptographicProcessor::new(
            blend_config.crypto.clone(),
            membership.clone(),
            rng.clone(),
        );
        let network_relay = overwatch_handle.relay::<NetworkService<_, _>>().await?;
        let network_adapter = Network::new(network_relay);

        // Incoming streams

        // Yields once every randomly-scheduled release round.
        let mut message_scheduler = UninitializedMessageScheduler::<
            _,
            _,
            ProcessedMessage<Network::BroadcastSettings>,
        >::new(
            blend_config.session_stream(),
            blend_config.scheduler_settings(),
            rng.clone(),
        )
        .wait_next_session_start()
        .await;

        // Yields new messages received via Blend peers.
        let mut blend_messages = backend.listen_to_incoming_messages();

        status_updater.notify_ready();
        tracing::info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>
        )
        .await?;

        loop {
            tokio::select! {
                Some(local_data_message) = inbound_relay.next() => {
                    handle_local_data_message(local_data_message, &mut cryptographic_processor, backend, &mut message_scheduler).await;
                }
                Some(incoming_message) = blend_messages.next() => {
                    handle_incoming_blend_message(incoming_message, &mut message_scheduler, &cryptographic_processor);
                }
                Some(round_info) = message_scheduler.next() => {
                    handle_release_round(round_info, &mut cryptographic_processor, &mut rng, backend, &network_adapter).await;
                }
            }
        }
    }
}

/// Blend a new message received from another service.
///
/// When a new local data message is received, an attempt to serialize and
/// encapsulate its payload is performed. If encapsulation is successful, the
/// message is sent over the Blend network and the Blend scheduler notified of
/// the new message sent.
/// These messages do not go through the Blend scheduler hence are not delayed,
/// as per the spec.
async fn handle_local_data_message<
    NodeId,
    Rng,
    Backend,
    SessionClock,
    BroadcastSettings,
    RuntimeServiceId,
>(
    local_data_message: ServiceMessage<BroadcastSettings>,
    cryptographic_processor: &mut CryptographicProcessor<NodeId, Rng>,
    backend: &Backend,
    scheduler: &mut MessageScheduler<SessionClock, Rng, ProcessedMessage<BroadcastSettings>>,
) where
    NodeId: Eq + Hash + Send,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId> + Sync,
    BroadcastSettings: Serialize,
{
    let ServiceMessage::Blend(message_payload) = local_data_message;

    let Ok(serialized_data_message) = wire::serialize(&message_payload).inspect_err(|_| tracing::error!(target: LOG_TARGET, "Message from internal service failed to be serialized.")) else {
        return;
    };

    let Ok(wrapped_message) = cryptographic_processor
        .encapsulate_data_payload(&serialized_data_message)
        .inspect_err(|e| {
            tracing::error!(target: LOG_TARGET, "Failed to wrap message: {e:?}");
        })
    else {
        return;
    };
    backend.publish(wrapped_message).await;
    scheduler.notify_new_data_message();
}

/// Processes an already unwrapped and validated Blend message received from
/// a core or edge peer.
fn handle_incoming_blend_message<Rng, NodeId, SessionClock, BroadcastSettings>(
    validated_encapsulated_message: EncapsulatedMessageWithValidatedPublicHeader,
    scheduler: &mut MessageScheduler<SessionClock, Rng, ProcessedMessage<BroadcastSettings>>,
    cryptographic_processor: &CryptographicProcessor<NodeId, Rng>,
) where
    BroadcastSettings: for<'de> Deserialize<'de>,
{
    let Ok(decapsulated_message) = cryptographic_processor.decapsulate_message(validated_encapsulated_message.into_inner()).inspect_err(|e| {
        tracing::debug!(target: LOG_TARGET, "Failed to decapsulate received message with error {e:?}");
    }) else {
        return;
    };
    match decapsulated_message {
        DecapsulationOutput::Completed(fully_decapsulated_message) => {
            match fully_decapsulated_message.into_components() {
                (PayloadType::Cover, _) => {
                    tracing::info!(target: LOG_TARGET, "Discarding received cover message.");
                }
                (PayloadType::Data, serialized_data_message) => {
                    tracing::debug!(target: LOG_TARGET, "Processing a fully decapsulated data message.");
                    if let Ok(deserialized_network_message) =
                        wire::deserialize::<NetworkMessage<BroadcastSettings>>(
                            serialized_data_message.as_ref(),
                        )
                    {
                        scheduler.schedule_message(deserialized_network_message.into());
                    } else {
                        tracing::debug!(target: LOG_TARGET, "Unrecognized data message from blend backend. Dropping.");
                    }
                }
            }
        }
        DecapsulationOutput::Incompleted(remaining_encapsulated_message) => {
            scheduler.schedule_message(remaining_encapsulated_message.into());
        }
    }
}

/// Reacts to a new release tick as returned by the scheduler.
///
/// When that happens, the previously processed messages (both encapsulated and
/// unencapsulated ones) as well as optionally a cover message are handled.
/// For unencapsulated messages, they are broadcasted to the rest of the network
/// using the configured network adapter. For encapsulated messages as well as
/// the optional cover message, they are forwarded to the rest of the connected
/// Blend peers.
async fn handle_release_round<NodeId, Rng, Backend, NetAdapter, RuntimeServiceId>(
    RoundInfo {
        cover_message_generation_flag,
        processed_messages,
    }: RoundInfo<ProcessedMessage<NetAdapter::BroadcastSettings>>,
    cryptographic_processor: &mut CryptographicProcessor<NodeId, Rng>,
    rng: &mut Rng,
    backend: &Backend,
    network_adapter: &NetAdapter,
) where
    NodeId: Eq + Hash,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId> + Sync,
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Sync,
{
    let mut processed_messages_relay_futures = processed_messages
        .into_iter()
        .map(
            |message_to_release| -> Box<dyn Future<Output = ()> + Send + Unpin> {
                match message_to_release {
                    ProcessedMessage::Network(NetworkMessage {
                        broadcast_settings,
                        message,
                    }) => Box::new(network_adapter.broadcast(message, broadcast_settings)),
                    ProcessedMessage::Encapsulated(encapsulated_message) => {
                        Box::new(backend.publish(*encapsulated_message))
                    }
                }
            },
        )
        .collect::<Vec<_>>();
    if cover_message_generation_flag.is_some() {
        let cover_message = cryptographic_processor
            .encapsulate_cover_payload(&random_sized_bytes::<{ size_of::<u32>() }>())
            .expect("Should not fail to generate new cover message");
        processed_messages_relay_futures.push(Box::new(backend.publish(cover_message)));
    }
    // TODO: If we send all of them in parallel, do we still need to shuffle them?
    processed_messages_relay_futures.shuffle(rng);
    let total_message_count = processed_messages_relay_futures.len();

    // Release all messages concurrently, and wait for all of them to be sent.
    join_all(processed_messages_relay_futures).await;
    tracing::debug!(target: LOG_TARGET, "Sent out {total_message_count} processed and/or cover messages at this release window.");
}

impl<Backend, NodeId, Network, RuntimeServiceId> Drop
    for BlendService<Backend, NodeId, Network, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    fn drop(&mut self) {
        tracing::info!(target: LOG_TARGET, "Shutting down Blend backend");
        self.backend.shutdown();
    }
}
