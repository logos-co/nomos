pub mod backends;
pub mod network;

use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

use async_trait::async_trait;
use backends::BlendBackend;
use futures::{future::join_all, Stream, StreamExt as _};
use network::NetworkAdapter;
use nomos_blend_message::{sphinx::SphinxMessage, BlendMessage, MessageUnwrapError};
use nomos_blend_scheduling::{
    cover_traffic::{CoverTraffic, CoverTrafficSettings},
    membership::{Membership, Node},
    message::BlendOutgoingMessage,
    message_blend::{
        crypto::CryptographicProcessor, temporal::TemporalScheduler,
        CryptographicProcessorSettings, MessageBlendExt as _, MessageBlendSettings,
    },
    message_scheduler::{
        round_info::RoundInfo,
        session_info::{Session, SessionInfo},
    },
    persistent_transmission::{
        PersistentTransmissionExt as _, PersistentTransmissionSettings,
        PersistentTransmissionStream,
    },
    UninitializedMessageScheduler,
};
use nomos_core::wire;
use nomos_network::NetworkService;
use nomos_utils::{
    bounded_duration::{MinimalBoundedDuration, SECOND},
    math::NonNegativeF64,
};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{seq::SliceRandom, SeedableRng as _};
use rand_chacha::ChaCha12Rng;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::{sync::mpsc, time};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};
use tracing::{debug, error};

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
                settings_reader.get_updated_settings(),
                service_resources_handle.overwatch_handle.clone(),
                blend_config.membership(),
                ChaCha12Rng::from_entropy(),
            ),
            service_resources_handle,
            membership: blend_config.membership(),
        })
    }

    #[expect(clippy::too_many_lines, reason = "This code will soon be refactored.")]
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
        let rng = ChaCha12Rng::from_entropy();
        let mut cryptographic_processor = CryptographicProcessor::new(
            blend_config.message_blend.cryptographic_processor.clone(),
            membership.clone(),
            rng.clone(),
        );
        let network_relay = overwatch_handle.relay::<NetworkService<_, _>>().await?;
        let network_adapter = Network::new(network_relay);

        // tier 1 persistent transmission
        let (persistent_sender, persistent_receiver) = mpsc::unbounded_channel();
        let mut persistent_transmission_messages: PersistentTransmissionStream<_, _> =
            UnboundedReceiverStream::new(persistent_receiver).persistent_transmission(
                IntervalStream::new(time::interval(Duration::from_secs_f64(
                    1.0 / blend_config.persistent_transmission.max_emission_frequency,
                )))
                .map(|_| ()),
            );

        // tier 2 blend
        let temporal_scheduler =
            TemporalScheduler::new(blend_config.message_blend.temporal_processor, rng.clone());
        let mut blend_messages = backend.listen_to_incoming_messages().blend(
            blend_config.message_blend.clone(),
            membership.clone(),
            temporal_scheduler,
            rng.clone(),
        );

        // tier 3 cover traffic
        let mut cover_traffic = CoverTraffic::new(
            blend_config.cover_traffic.cover_traffic_settings(
                &blend_config.timing_settings,
                &blend_config.message_blend.cryptographic_processor,
            ),
            blend_config
                .timing_settings
                .session_stream(membership.size()),
            rng,
        )
        .wait_ready()
        .await;

        // local messages are bypassed and sent immediately
        let mut local_messages = inbound_relay.map(|ServiceMessage::Blend(message)| {
            wire::serialize(&message)
                .expect("Message from internal services should not fail to serialize")
        });

        let mut message_scheduler = UninitializedMessageScheduler::new(
            IntervalStream::new(time::interval(Duration::from_secs(
                blend_config.timing_settings.rounds_per_session.get()
                    * blend_config.timing_settings.round_duration.as_secs(),
            )))
            .enumerate()
            .map(|(i, _)| SessionInfo {
                session_number: Session::from(i.into()),
                // TODO: Calculate right one
                core_quota: 100,
            }),
            blend_config
                .message_scheduler
                .message_scheduler_settings(&blend_config.timing_settings),
            rng,
        )
        .wait_next_session_start()
        .await;

        status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>
        )
        .await?;

        // Temporary structure used to distinguish the messages that are scheduled via
        // the temporal scheduler. We keep track of locally generated messages
        // because they affect the schedule of the cover message scheduler. This
        // logic will find a better place after we refactor the code to implement the
        // latest v1 of the spec.
        let mut scheduled_local_messages = HashSet::new();
        loop {
            // We listen for:
            // * 1) Local messages (i.e., block production messages)
            //     * They are wrapped and broadcasted immediately.
            // * 2) Blend Messages received by peers
            //     * They are immediately re-blended (minus the sender), processed, and then
            //       scheduled for the next release window
            // * 3) Scheduler release rounds
            //     * At each round, the scheduler can release a new set of messages, which
            //       can include a cover message and/or the set of messages processed since
            //       the last release window.
            tokio::select! {
                // Point 1.
                Some(local_message) = local_messages.next() => {
                    let Ok(wrapped_message) = cryptographic_processor.wrap_message(&local_message).inspect_err(|e| error!("Failed to wrap message: {local_message:?} with error {e:?}")) else {
                        continue;
                    };
                    backend.publish(wrapped_message).await;
                    message_scheduler.notify_new_data_message();
                }
                // Point 2.
                Some(incoming_blend_message) = blend_messages.next() => {
                    // TODO: Add message header validation.
                    match cryptographic_processor.unwrap_message(incoming_blend_message.as_ref()) {
                        Ok((unwrapped_message, fully_unwrapped)) => {
                            if fully_unwrapped {
                                debug!("Processing a fully unwrapped message.");
                                // TODO: Change deserialization logic to return the actual type of message to the service, instead of assuming that a failed deserialization can mean a cover message as well as a malformed message.
                                if wire::deserialize::<NetworkMessage<Network::BroadcastSettings>>(&unwrapped_message).is_ok() {
                                    // Message is a valid network message, we add it (still serialized) to the release queue. We will deserialize it again later before releasing.
                                    message_scheduler.schedule_message(BlendOutgoingMessage::FullyUnwrapped(unwrapped_message.into()).into());
                                } else {
                                    // Message failed to be deserialized. It means that it was either malformed, or a cover message.
                                    debug!("Unrecognized message from blend backend. Either malformed or a cover message. Dropping.");
                                };
                            } else {
                                message_scheduler.schedule_message(BlendOutgoingMessage::Outbound(unwrapped_message.into()).into());
                            }
                        }
                        Err(e @ MessageUnwrapError::NotAllowed) => {
                            debug!("{e}");
                        }
                        Err(e) => {
                            error!("Failed to unwrap message: {e}");
                        }
                    }
                }
                // Point 3.
                Some(RoundInfo { mut processed_messages, cover_message }) = message_scheduler.next() => {
                    enum MessageType {
                        FullyUnwrapped,
                        Wrapped,
                        Cover
                    }
                    let mut flagged_messages: Vec<_> = processed_messages.into_iter().map(|m| {
                        match m {
                            BlendOutgoingMessage::Outbound(outbound_message) => {
                                (Vec::from(outbound_message), MessageType::Wrapped)
                            },
                            BlendOutgoingMessage::FullyUnwrapped(fully_unwrapped_message) => {
                                (Vec::from(fully_unwrapped_message), MessageType::FullyUnwrapped)
                            },
                        }
                     }).chain(cover_message.map(|c| (Vec::from(c), MessageType::Cover))).collect();
                    flagged_messages.shuffle(&mut rng);

                    // Process all messages in parallel.
                    join_all(flagged_messages.into_iter().map(|(raw_message, message_type)| async move {
                        match message_type {
                            MessageType::FullyUnwrapped => {
                                if let Ok(deserialized_network_message) = wire::deserialize::<NetworkMessage<Network::BroadcastSettings>>(&raw_message) {
                                    network_adapter.broadcast(deserialized_network_message.message, deserialized_network_message.broadcast_settings).await;
                                } else {
                                    error!("Failed to deserialize previously validated network message: {raw_message:?}");
                                }
                            }
                            MessageType::Wrapped | MessageType::Cover => {
                                backend.publish(raw_message).await;
                            }
                        }
                    })).await;
                }
            }
        }
    }
}

impl<Backend, Network, RuntimeServiceId> Drop for BlendService<Backend, Network, RuntimeServiceId>
where
    Backend: BlendBackend<RuntimeServiceId> + 'static,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    fn drop(&mut self) {
        tracing::info!("Shutting down Blend backend");
        self.backend.shutdown();
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
    ) -> Option<Vec<u8>> {
        match cryptographic_processor.wrap_message(message) {
            Ok(wrapped_message) => {
                if let Err(e) = persistent_sender.send(wrapped_message.clone()) {
                    tracing::error!("Error sending message to persistent stream: {e}");
                }
                Some(wrapped_message)
            }
            Err(e) => {
                tracing::error!("Failed to wrap message: {:?}", e);
                None
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, BackendNodeId> {
    pub backend: BackendSettings,
    pub message_blend: MessageBlendSettings<SphinxMessage>,
    pub cover_traffic: CoverTrafficExtSettings,
    pub message_scheduler: MessageSchedulerExt,
    #[serde(flatten)]
    pub timing_settings: TimingSettings,
    pub membership: Vec<Node<BackendNodeId, <SphinxMessage as BlendMessage>::PublicKey>>,
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TimingSettings {
    pub rounds_per_session: NonZeroU64,
    pub rounds_per_interval: NonZeroUsize,
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub round_duration: Duration,
    pub rounds_per_observation_window: NonZeroUsize,
}

impl TimingSettings {
    fn session_stream(
        &self,
        membership_size: usize,
    ) -> Box<dyn Stream<Item = SessionInfo> + Send + Unpin> {
        let session_duration_in_seconds = self
            .round_duration
            .as_secs()
            .checked_mul(self.rounds_per_session.get())
            .expect("Overflow when computing the total duration of a session in seconds.");
        Box::new(
            IntervalStream::new(time::interval(Duration::from_secs(
                session_duration_in_seconds,
            )))
            .enumerate()
            .map(move |(i, _)| SessionInfo {
                session_number: (i as u64).into(),
                membership_size,
            }),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoverTrafficExtSettings {
    pub message_frequency_per_round: NonNegativeF64,
    pub redundancy_parameter: usize,
    pub intervals_for_safety_buffer: u64,
}

impl CoverTrafficExtSettings {
    const fn cover_traffic_settings(
        &self,
        timing_settings: &TimingSettings,
        cryptographic_processor_settings: &CryptographicProcessorSettings<
            <SphinxMessage as BlendMessage>::PrivateKey,
        >,
    ) -> CoverTrafficSettings {
        CoverTrafficSettings {
            blending_ops_per_message: cryptographic_processor_settings.num_blend_layers,
            message_frequency_per_round: self.message_frequency_per_round,
            redundancy_parameter: self.redundancy_parameter,
            round_duration: timing_settings.round_duration,
            rounds_per_interval: timing_settings.rounds_per_interval,
            rounds_per_session: timing_settings.rounds_per_session,
            intervals_for_safety_buffer: self.intervals_for_safety_buffer,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MessageSchedulerExt {
    pub additional_safety_intervals: usize,
    pub maximum_release_delay_in_rounds: NonZeroUsize,
}

impl MessageSchedulerExt {
    pub fn message_scheduler_settings(
        &self,
        timing_settings: &TimingSettings,
    ) -> nomos_blend_scheduling::message_scheduler::Settings {
        nomos_blend_scheduling::message_scheduler::Settings {
            additional_safety_intervals: self.additional_safety_intervals,
            expected_intervals_per_session: NonZeroUsize::try_from(
                timing_settings.rounds_per_session.get() as usize
                    / timing_settings.rounds_per_interval.get(),
            )
            .expect("Calculated intervals per sessions cannot be zero."),
            maximum_release_delay_in_rounds: self.maximum_release_delay_in_rounds,
            round_duration: timing_settings.round_duration,
            rounds_per_interval: timing_settings.rounds_per_interval,
        }
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
