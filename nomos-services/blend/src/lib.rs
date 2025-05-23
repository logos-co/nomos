pub mod backends;
pub mod network;

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    time::Duration,
};

use async_trait::async_trait;
use backends::BlendBackend;
use futures::StreamExt as _;
use network::NetworkAdapter;
use nomos_blend::{
    cover_traffic::{CoverTraffic, CoverTrafficSettings},
    membership::{Membership, Node},
    message_blend::{
        crypto::CryptographicProcessor, temporal::TemporalScheduler,
        CryptographicProcessorSettings, MessageBlendExt as _, MessageBlendSettings,
    },
    persistent_transmission::{
        PersistentTransmissionExt as _, PersistentTransmissionSettings,
        PersistentTransmissionStream,
    },
    BlendOutgoingMessage,
};
use nomos_blend_message::{sphinx::SphinxMessage, BlendMessage};
use nomos_core::wire;
use nomos_network::NetworkService;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::SeedableRng as _;
use rand_chacha::ChaCha12Rng;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{sync::mpsc, time};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};

/// A blend service that sends messages to the blend network
/// and broadcasts fully unwrapped messages through the [`NetworkService`].
///
/// The blend backend and the network adapter are generic types that are
/// independent of each other. For example, the blend backend can use the
/// libp2p network stack, while the network adapter can use the other network
/// backend.
pub struct BlendService<Backend, Network, RuntimeServiceId>
where
    Backend: BlendBackend<RuntimeServiceId> + 'static,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: Membership<Backend::NodeId, SphinxMessage>,
}

impl<Backend, Network, RuntimeServiceId> ServiceData
    for BlendService<Backend, Network, RuntimeServiceId>
where
    Backend: BlendBackend<RuntimeServiceId> + 'static,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    type Settings = BlendConfig<Backend::Settings, Backend::NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<Network::BroadcastSettings>;
}

#[async_trait]
impl<Backend, Network, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<Backend, Network, RuntimeServiceId>
where
    Backend: BlendBackend<RuntimeServiceId> + Send + 'static,
    Backend::NodeId: Hash + Eq + Unpin,
    Network: NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
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
        Ok(Self {
            backend: <Backend as BlendBackend<RuntimeServiceId>>::new(
                settings_reader.get_updated_settings().backend,
                service_resources_handle.overwatch_handle.clone(),
                blend_config.membership(),
                ChaCha12Rng::from_entropy(),
            ),
            service_resources_handle,
            membership: blend_config.membership(),
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle,
            mut backend,
            membership,
        } = self;
        let blend_config = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let mut cryptographic_processor = CryptographicProcessor::new(
            blend_config.message_blend.cryptographic_processor.clone(),
            membership.clone(),
            ChaCha12Rng::from_entropy(),
        );
        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await?;
        let network_adapter = Network::new(network_relay);

        // tier 1 persistent transmission
        let (persistent_sender, persistent_receiver) = mpsc::unbounded_channel();
        let mut persistent_transmission_messages: PersistentTransmissionStream<_, _, _> =
            UnboundedReceiverStream::new(persistent_receiver).persistent_transmission(
                blend_config.persistent_transmission,
                ChaCha12Rng::from_entropy(),
                IntervalStream::new(time::interval(Duration::from_secs_f64(
                    1.0 / blend_config.persistent_transmission.max_emission_frequency,
                )))
                .map(|_| ()),
                SphinxMessage::DROP_MESSAGE.to_vec(),
            );

        // tier 2 blend
        let temporal_scheduler = TemporalScheduler::new(
            blend_config.message_blend.temporal_processor,
            ChaCha12Rng::from_entropy(),
        );
        let mut blend_messages = backend.listen_to_incoming_messages().blend(
            blend_config.message_blend.clone(),
            membership.clone(),
            temporal_scheduler,
            ChaCha12Rng::from_entropy(),
        );

        // tier 3 cover traffic
        let mut cover_traffic: CoverTraffic<_, _, SphinxMessage> = CoverTraffic::new(
            blend_config.cover_traffic.cover_traffic_settings(
                &membership,
                &blend_config.message_blend.cryptographic_processor,
            ),
            blend_config.cover_traffic.epoch_stream(),
            blend_config.cover_traffic.slot_stream(),
        );

        // local messages are bypassed and sent immediately
        let mut local_messages =
            service_resources_handle
                .inbound_relay
                .map(|ServiceMessage::Blend(message)| {
                    wire::serialize(&message)
                        .expect("Message from internal services should not fail to serialize")
                });

        loop {
            tokio::select! {
                Some(msg) = persistent_transmission_messages.next() => {
                    backend.publish(msg).await;
                }
                // Already processed blend messages
                Some(msg) = blend_messages.next() => {
                    match msg {
                        // If message is not fully unwrapped, forward the remaining layers to the next hop.
                        BlendOutgoingMessage::Outbound(msg) => {
                            if let Err(e) = persistent_sender.send(msg) {
                                tracing::error!("Error sending message to persistent stream: {e}");
                            }
                        }
                        // If the message is fully unwrapped, broadcast it (unencrypted) to the rest of the network.
                        BlendOutgoingMessage::FullyUnwrapped(msg) => {
                            tracing::debug!("Broadcasting fully unwrapped message");
                            match wire::deserialize::<NetworkMessage<Network::BroadcastSettings>>(&msg) {
                                Ok(msg) => {
                                    network_adapter.broadcast(msg.message, msg.broadcast_settings).await;
                                },
                                _ => {
                                    tracing::debug!("unrecognized message from blend backend");
                                }
                            }
                        }
                    }
                }
                Some(msg) = cover_traffic.next() => {
                    Self::wrap_and_send_to_persistent_transmission(&msg, &mut cryptographic_processor, &persistent_sender);
                }
                Some(msg) = local_messages.next() => {
                    Self::wrap_and_send_to_persistent_transmission(&msg, &mut cryptographic_processor, &persistent_sender);
                }
            }
        }
    }
}

impl<Backend, Network, RuntimeServiceId> BlendService<Backend, Network, RuntimeServiceId>
where
    Backend: BlendBackend<RuntimeServiceId> + Send + 'static,
    Backend::Settings: Clone,
    Backend::NodeId: Hash + Eq,
    Network: NetworkAdapter<RuntimeServiceId>,
    Network::BroadcastSettings: Clone + Debug + Serialize + DeserializeOwned,
{
    fn wrap_and_send_to_persistent_transmission(
        message: &[u8],
        cryptographic_processor: &mut CryptographicProcessor<
            Backend::NodeId,
            ChaCha12Rng,
            SphinxMessage,
        >,
        persistent_sender: &mpsc::UnboundedSender<Vec<u8>>,
    ) {
        match cryptographic_processor.wrap_message(message) {
            Ok(wrapped_message) => {
                if let Err(e) = persistent_sender.send(wrapped_message) {
                    tracing::error!("Error sending message to persistent stream: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Failed to wrap message: {:?}", e);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, BackendNodeId> {
    pub backend: BackendSettings,
    pub message_blend: MessageBlendSettings<SphinxMessage>,
    pub persistent_transmission: PersistentTransmissionSettings,
    pub cover_traffic: CoverTrafficExtSettings,
    pub membership: Vec<Node<BackendNodeId, <SphinxMessage as BlendMessage>::PublicKey>>,
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoverTrafficExtSettings {
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub epoch_duration: Duration,
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub slot_duration: Duration,
}

impl CoverTrafficExtSettings {
    const fn cover_traffic_settings<NodeId>(
        &self,
        membership: &Membership<NodeId, SphinxMessage>,
        cryptographic_processor_settings: &CryptographicProcessorSettings<
            <SphinxMessage as BlendMessage>::PrivateKey,
        >,
    ) -> CoverTrafficSettings
    where
        NodeId: Hash + Eq,
    {
        CoverTrafficSettings {
            node_id: membership.local_node().public_key,
            number_of_hops: cryptographic_processor_settings.num_blend_layers,
            slots_per_epoch: self.slots_per_epoch(),
            network_size: membership.size(),
        }
    }

    const fn slots_per_epoch(&self) -> usize {
        (self.epoch_duration.as_secs() as usize)
            .checked_div(self.slot_duration.as_secs() as usize)
            .expect("Invalid epoch & slot duration")
    }

    fn epoch_stream(
        &self,
    ) -> futures::stream::Map<
        futures::stream::Enumerate<IntervalStream>,
        impl FnMut((usize, time::Instant)) -> usize,
    > {
        IntervalStream::new(time::interval(self.epoch_duration))
            .enumerate()
            .map(|(i, _)| i)
    }

    fn slot_stream(
        &self,
    ) -> futures::stream::Map<
        futures::stream::Enumerate<IntervalStream>,
        impl FnMut((usize, time::Instant)) -> usize,
    > {
        let slots_per_epoch = self.slots_per_epoch();
        IntervalStream::new(time::interval(self.slot_duration))
            .enumerate()
            .map(move |(i, _)| i % slots_per_epoch)
    }
}

impl<BackendSettings, BackendNodeId> BlendConfig<BackendSettings, BackendNodeId>
where
    BackendNodeId: Clone + Hash + Eq,
{
    fn membership(&self) -> Membership<BackendNodeId, SphinxMessage> {
        let public_key = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(
            self.message_blend.cryptographic_processor.private_key,
        ))
        .to_bytes();
        Membership::new(self.membership.clone(), &public_key)
    }
}

/// A message that is handled by [`BlendService`].
#[derive(Debug)]
pub enum ServiceMessage<BroadcastSettings> {
    /// To send a message to the blend network and eventually broadcast it to
    /// the [`NetworkService`].
    Blend(NetworkMessage<BroadcastSettings>),
}

/// A message that is sent to the blend network.
///
/// To eventually broadcast the message to the network service,
/// [`BroadcastSettings`] must be included in the [`NetworkMessage`].
/// [`BroadcastSettings`] is a generic type defined by [`NetworkAdapter`].
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkMessage<BroadcastSettings> {
    pub message: Vec<u8>,
    pub broadcast_settings: BroadcastSettings,
}
