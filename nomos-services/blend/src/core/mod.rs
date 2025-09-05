pub mod backends;
mod handlers;
pub mod network;
pub mod settings;

use std::{
    fmt::{Debug, Display},
    future::Future,
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use backends::BlendBackend;
use futures::{future::join_all, Stream, StreamExt as _};
use network::NetworkAdapter;
use nomos_blend_message::{crypto::random_sized_bytes, encap::DecapsulationOutput, PayloadType};
use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{crypto::CryptographicProcessor, CryptographicProcessorSettings},
    message_scheduler::{round_info::RoundInfo, MessageScheduler},
    session::{SessionEvent, SessionEventStream},
    UninitializedMessageScheduler,
};
use nomos_core::wire;
use nomos_network::NetworkService;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{seq::SliceRandom as _, RngCore, SeedableRng as _};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::{sync::mpsc, time::timeout};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};

use crate::{
    core::{
        handlers::{Error, MessageHandler},
        settings::BlendConfig,
    },
    membership,
    message::{NetworkMessage, ProcessedMessage, ServiceMessage},
    settings::constant_membership_stream,
};

pub(super) mod service_components;

const LOG_TARGET: &str = "blend::service::core";

/// A blend service that sends messages to the blend network
/// and broadcasts fully unwrapped messages through the [`NetworkService`].
///
/// The blend backend and the network adapter are generic types that are
/// independent of each other. For example, the blend backend can use the
/// libp2p network stack, while the network adapter can use the other network
/// backend.
pub struct BlendService<Backend, NodeId, Network, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<MembershipAdapter>,
}

impl<Backend, NodeId, Network, MembershipAdapter, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, Network, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    type Settings = BlendConfig<Backend::Settings, NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<Network::BroadcastSettings>;
}

const CHANNEL_SIZE: usize = 10;

#[async_trait]
impl<Backend, NodeId, Network, MembershipAdapter, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, Network, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Send + Eq + Hash + Sync + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync,
    MembershipAdapter: membership::Adapter + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    <MembershipAdapter as membership::Adapter>::Error: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<NetworkService<Network::Backend, RuntimeServiceId>>
        + AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
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
        Ok(Self {
            service_resources_handle,
            _phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    inbound_relay,
                    overwatch_handle,
                    settings_handle,
                    status_updater,
                    ..
                },
            ..
        } = self;

        let blend_config = settings_handle.notifier().get_updated_settings();

        let network_relay = overwatch_handle.relay::<NetworkService<_, _>>().await?;
        let network_adapter = Network::new(network_relay);

        let _membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            blend_config.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;
        // TODO: Use membership_stream once the membership/SDP services are ready to provide the real membership: https://github.com/logos-co/nomos/issues/1532

        let session_stream = SessionEventStream::new(
            Box::pin(constant_membership_stream(
                blend_config.membership(),
                blend_config.time.session_duration(),
            )),
            blend_config.time.session_transition_period(),
        );

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>,
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        run::<Backend, _, _, _>(
            session_stream,
            inbound_relay,
            network_adapter,
            &blend_config,
            &overwatch_handle,
            || {
                status_updater.notify_ready();
                info!(
                    target: LOG_TARGET,
                    "Service '{}' is ready.",
                    <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
                );
            },
        )
        .await
    }
}

async fn run<Backend, NodeId, Network, RuntimeServiceId>(
    mut session_stream: impl Stream<Item = SessionEvent<Membership<NodeId>>> + Send + Unpin,
    mut local_data_messages: impl Stream<Item = ServiceMessage<Network::BroadcastSettings>>
        + Send
        + Unpin,
    network_adapter: Network,
    settings: &Settings<Backend, BlakeRng, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    notify_ready: impl Fn(),
) -> Result<(), overwatch::DynError>
where
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Send + Eq + Hash + Sync + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync,
    RuntimeServiceId: Clone,
{
    // Read the initial membership, expecting it to be yielded immediately.
    // We use 1s timeout to tolerate small delays.
    let SessionEvent::NewSession(membership) =
        timeout(Duration::from_secs(1), session_stream.next())
            .await
            .expect("Session stream should yield the first event immediately")
            .expect("Session stream shouldn't be closed")
    else {
        panic!("NewSession must be yielded first");
    };

    let mut handler = MessageHandler::try_new_with_core_condition_check(
        &settings.crypto,
        settings.minimum_network_size.get() as usize,
        membership,
    )?;

    let (session_event_sender, session_event_receiver) =
        mpsc::channel::<SessionEvent<Membership<NodeId>>>(CHANNEL_SIZE);
    let mut backend = <Backend as BlendBackend<NodeId, BlakeRng, RuntimeServiceId>>::new(
        settings.clone(),
        overwatch_handle.clone(),
        // TODO: check if it's okay to use this stream that already yielded the first item.
        Box::pin(ReceiverStream::new(session_event_receiver)),
        BlakeRng::from_entropy(),
    );
    // Yields new messages received via Blend peers.
    let mut incoming_blend_messages = backend.listen_to_incoming_messages();

    // Yields once every randomly-scheduled release round.
    let mut message_scheduler =
        UninitializedMessageScheduler::<_, _, ProcessedMessage<Network::BroadcastSettings>>::new(
            // TODO: verify if this is correct
            settings.session_stream(),
            settings.scheduler_settings(),
            BlakeRng::from_entropy(),
        )
        .wait_next_session_start()
        .await;

    notify_ready();

    loop {
        tokio::select! {
            Some(local_data_message) = local_data_messages.next() => {
                handler.handle_local_data_message(local_data_message, &backend, &mut message_scheduler).await;
            }
            Some(incoming_message) = incoming_blend_messages.next() => {
                handler.handle_incoming_blend_message(incoming_message, &mut message_scheduler);
            }
            Some(round_info) = message_scheduler.next() => {
                handler.handle_release_round(round_info, &backend, &network_adapter).await;
            }
            Some(event) = session_stream.next() => {
                handler = handle_session_event(event, handler, &session_event_sender, &settings.crypto, settings.minimum_network_size.get() as usize).await?;
            }
        }
    }
}

type Settings<Backend, Rng, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, Rng, RuntimeServiceId>>::Settings, NodeId>;

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
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Sync,
    BroadcastSettings: Serialize,
{
    let ServiceMessage::Blend(message_payload) = local_data_message;

    let Ok(serialized_data_message) = wire::serialize(&message_payload).inspect_err(
        |_| error!(target: LOG_TARGET, "Message from internal service failed to be serialized."),
    ) else {
        return;
    };

    let Ok(wrapped_message) = cryptographic_processor
        .encapsulate_data_payload(&serialized_data_message)
        .inspect_err(|e| {
            error!(target: LOG_TARGET, "Failed to wrap message: {e:?}");
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
    let Ok(decapsulated_message) = cryptographic_processor
        .decapsulate_message(validated_encapsulated_message.into_inner())
        .inspect_err(|e| {
            debug!(target: LOG_TARGET, "Failed to decapsulate received message with error {e:?}");
        })
    else {
        return;
    };
    match decapsulated_message {
        DecapsulationOutput::Completed(fully_decapsulated_message) => {
            match fully_decapsulated_message.into_components() {
                (PayloadType::Cover, _) => {
                    info!(target: LOG_TARGET, "Discarding received cover message.");
                }
                (PayloadType::Data, serialized_data_message) => {
                    debug!(target: LOG_TARGET, "Processing a fully decapsulated data message.");
                    if let Ok(deserialized_network_message) =
                        wire::deserialize::<NetworkMessage<BroadcastSettings>>(
                            serialized_data_message.as_ref(),
                        )
                    {
                        scheduler.schedule_message(deserialized_network_message.into());
                    } else {
                        debug!(target: LOG_TARGET, "Unrecognized data message from blend backend. Dropping.");
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
    Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Sync,
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
    debug!(target: LOG_TARGET, "Sent out {total_message_count} processed and/or cover messages at this release window.");
}

/// Handles a new [`SessionEvent`] and forwards it to the given channel.
///
/// It returns a new [`Membership`] and a new [`CryptographicProcessor`]
/// if the event is a [`SessionEvent::NewSession`].
/// Otherwise, it returns [`None`].
async fn handle_session_event<NodeId>(
    event: SessionEvent<Membership<NodeId>>,
    message_handler: MessageHandler<NodeId>,
    forwarder: &mpsc::Sender<SessionEvent<Membership<NodeId>>>,
    crypto_settings: &CryptographicProcessorSettings,
    minimmal_network_size: usize,
) -> Result<MessageHandler<NodeId>, Error>
where
    NodeId: Clone + Eq + Hash + Send,
{
    let new_message_handler = if let SessionEvent::NewSession(ref membership) = event {
        MessageHandler::try_new_with_core_condition_check(
            crypto_settings,
            minimmal_network_size,
            membership.clone(),
        )?
    } else {
        message_handler
    };

    forwarder
        .send(event)
        .await
        .expect("Channel shouldn't be closed");

    Ok(new_message_handler)
}
