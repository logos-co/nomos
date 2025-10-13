pub mod backends;
pub mod network;
mod processor;
pub mod settings;

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use backends::BlendBackend;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use fork_stream::StreamExt as _;
use futures::{FutureExt as _, Stream, StreamExt as _, future::join_all, stream::pending};
use groth16::Field as _;
use network::NetworkAdapter;
use nomos_blend_message::{
    PayloadType,
    crypto::{
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::inputs::prove::{
                private::ProofOfCoreQuotaInputs,
                public::{CoreInputs, LeaderInputs},
            },
        },
        random_sized_bytes,
    },
    encap::{ProofsVerifier as ProofsVerifierTrait, decapsulated::DecapsulationOutput},
};
use nomos_blend_scheduling::{
    message_blend::{
        crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
        provers::core_and_leader::CoreAndLeaderProofsGenerator,
    },
    message_scheduler::{MessageScheduler, round_info::RoundInfo},
    session::{SessionEvent, UninitializedSessionEventStream},
    stream::UninitializedFirstReadyStream,
};
use nomos_core::{
    codec::{DeserializeOp as _, SerializeOp as _},
    crypto::ZkHash,
};
use nomos_network::NetworkService;
use nomos_time::{TimeService, TimeServiceMessage};
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use rand::{RngCore, SeedableRng as _, seq::SliceRandom as _};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot;
use tracing::info;

use crate::{
    core::{
        backends::SessionInfo,
        processor::{CoreCryptographicProcessor, Error},
        settings::BlendConfig,
    },
    epoch_info::{
        EpochEvent, EpochHandler, LeaderInputsMinusQuota, PolInfoProvider as PolInfoProviderTrait,
    },
    membership,
    message::{NetworkMessage, ProcessedMessage, ServiceMessage},
    session::CoreSessionInfo,
    settings::FIRST_STREAM_ITEM_READY_TIMEOUT,
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
pub struct BlendService<
    Backend,
    NodeId,
    Network,
    MembershipAdapter,
    ProofsGenerator,
    ProofsVerifier,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(
        Backend,
        MembershipAdapter,
        ProofsGenerator,
        TimeBackend,
        ChainService,
        PolInfoProvider,
    )>,
}

impl<
    Backend,
    NodeId,
    Network,
    MembershipAdapter,
    ProofsGenerator,
    ProofsVerifier,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> ServiceData
    for BlendService<
        Backend,
        NodeId,
        Network,
        MembershipAdapter,
        ProofsGenerator,
        ProofsVerifier,
        TimeBackend,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    type Settings = BlendConfig<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<Network::BroadcastSettings>;
}

#[async_trait]
impl<
    Backend,
    NodeId,
    Network,
    MembershipAdapter,
    ProofsGenerator,
    ProofsVerifier,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for BlendService<
        Backend,
        NodeId,
        Network,
        MembershipAdapter,
        ProofsGenerator,
        ProofsVerifier,
        TimeBackend,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Send + Eq + Hash + Sync + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync,
    MembershipAdapter: membership::Adapter<NodeId = NodeId, Error: Send + Sync + 'static> + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    ProofsGenerator: CoreAndLeaderProofsGenerator + Send,
    ProofsVerifier: ProofsVerifierTrait + Clone + Send,
    TimeBackend: nomos_time::backends::TimeBackend + Send,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync>,
    PolInfoProvider: PolInfoProviderTrait<RuntimeServiceId, Stream: Send + Unpin + 'static> + Send,
    RuntimeServiceId: AsServiceId<NetworkService<Network::Backend, RuntimeServiceId>>
        + AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
        + AsServiceId<ChainService>
        + AsServiceId<Self>
        + Clone
        + Debug
        + Display
        + Sync
        + Send
        + Unpin
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

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
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
            ..
        } = self;

        let blend_config = settings_handle.notifier().get_updated_settings();

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>,
            TimeService<_, _>,
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        let network_adapter = async {
            let network_relay = overwatch_handle.relay::<NetworkService<_, _>>().await?;
            Network::new(network_relay)
        }
        .await;

        let session_stream = async {
            let membership_stream = MembershipAdapter::new(
                overwatch_handle
                    .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                    .await
                    .expect("Failed to get relay channel with membership service."),
                blend_config.crypto.non_ephemeral_signing_key.public_key(),
            )
            .subscribe()
            .await
            .expect("Failed to get membership stream from membership service.");

            membership_stream.map(|membership| {
                let membership_size = membership.size();
                CoreSessionInfo {
                    membership,
                    // TODO: Replace with actual session value
                    session: 10,
                    poq_core_public_inputs: CoreInputs {
                        // TODO: Replace with actual membership returned value
                        zk_root: ZkHash::ZERO,
                        quota: blend_config.session_quota(membership_size),
                    },
                    // TODO: Replace with actual ZK key and merkle path calculation.
                    poq_core_private_inputs: ProofOfCoreQuotaInputs {
                        core_path: vec![],
                        core_path_selectors: vec![],
                        core_sk: ZkHash::ZERO,
                    },
                }
            })
        }
        .await;

        let mut epoch_handler = async {
            let chain_service = CryptarchiaServiceApi::<ChainService, _>::new(&overwatch_handle)
                .await
                .expect("Failed to establish channel with chain service.");
            EpochHandler::new(
                chain_service,
                blend_config.time.epoch_transition_period_in_slots,
            )
        }
        .await;

        let clock_stream = async {
            let time_relay = overwatch_handle
                .relay::<TimeService<_, _>>()
                .await
                .expect("Relay with time service should be available.");
            let (sender, receiver) = oneshot::channel();
            time_relay
                .send(TimeServiceMessage::Subscribe { sender })
                .await
                .expect("Failed to subscribe to slot clock.");
            receiver
                .await
                .expect("Should not fail to receive slot stream from time service.")
        }
        .await;

        let (current_session_info, session_stream) = UninitializedSessionEventStream::new(
            session_stream,
            FIRST_STREAM_ITEM_READY_TIMEOUT,
            blend_config.time.session_transition_period(),
        )
        .await_first_ready()
        .await
        .expect("The current session info must be available.");

        let session_stream = session_stream.fork();

        let (mut current_epoch_info, mut epoch_stream) = async {
            let (first_tick, epoch_stream) =
                UninitializedFirstReadyStream::new(clock_stream, FIRST_STREAM_ITEM_READY_TIMEOUT)
                    .first()
                    .await
                    .expect("The node clock must be ready and must yield the first slot.");
            let epoch_state = epoch_handler
                .tick(first_tick)
                .await
                .expect("The epoch state for the first tick must be available.");
            let EpochEvent::NewEpoch(new_epoch_info) = epoch_state else {
                panic!("First tick expected to be a new epoch info.");
            };
            (new_epoch_info, epoch_stream)
        }
        .await;

        info!(
            target: LOG_TARGET,
            "The current membership is ready: {} nodes.",
            current_session_info.membership.size()
        );

        let public_inputs = PoQVerificationInputsMinusSigningKey {
            core: current_session_info.poq_core_public_inputs,
            leader: LeaderInputs {
                message_quota: blend_config.crypto.num_blend_layers,
                pol_epoch_nonce: current_epoch_info.pol_epoch_nonce,
                pol_ledger_aged: current_epoch_info.pol_ledger_aged,
                total_stake: current_epoch_info.total_stake,
            },
            session: current_session_info.session,
        };
        let mut crypto_processor =
            CoreCryptographicProcessor::<_, ProofsGenerator, _>::try_new_with_core_condition_check(
                current_session_info.membership.clone(),
                blend_config.minimum_network_size,
                &blend_config.crypto,
                public_inputs,
                current_session_info.poq_core_private_inputs,
            )
            .expect("The initial membership should satisfy the core node condition");

        let mut message_scheduler = {
            let membership_info_stream =
                map_session_event_stream(session_stream.clone(), |(membership, _)| membership);
            let (initial_scheduler_session_info, scheduler_session_info_stream) = blend_config
                .session_info_stream(&current_session_info.membership, membership_info_stream);

            MessageScheduler::<_, _, ProcessedMessage<Network::BroadcastSettings>>::new(
                scheduler_session_info_stream,
                initial_scheduler_session_info,
                BlakeRng::from_entropy(),
                blend_config.scheduler_settings(),
            )
        };

        let mut backend =
            <Backend as BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>>::new(
                blend_config.clone(),
                overwatch_handle.clone(),
                SessionInfo {
                    membership: current_session_info.membership,
                    poq_verification_inputs: public_inputs,
                },
                // TODO: Replace with actual combined stream.
                pending().boxed(),
                BlakeRng::from_entropy(),
            );

        // Yields new messages received via Blend peers.
        let mut blend_messages = backend.listen_to_incoming_messages();

        // Rng for releasing messages.
        let mut rng = BlakeRng::from_entropy();

        status_updater.notify_ready();
        tracing::info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        // There might be services that depend on Blend to be ready before starting, so
        // we cannot wait for the stream to be sent before we signal we are
        // ready, hence this should always be called after `notify_ready();`.
        // Also, Blend services start even if such a stream is not immediately
        // available, since they will simply keep blending cover messages.
        let secret_pol_info_stream = PolInfoProvider::subscribe(&overwatch_handle)
            .await
            .expect("Should not fail to subscribe to secret PoL info stream.");

        loop {
            tokio::select! {
                Some(local_data_message) = inbound_relay.next() => {
                    handle_local_data_message(local_data_message, &mut crypto_processor, &backend, &mut message_scheduler).await;
                }
                Some(incoming_message) = blend_messages.next() => {
                    handle_incoming_blend_message(incoming_message, &mut message_scheduler, &crypto_processor);
                }
                Some(round_info) = message_scheduler.next() => {
                    handle_release_round(round_info, &mut crypto_processor, &mut rng, &backend, &network_adapter).await;
                }
                Some(tick) = clock_stream.next() => {
                    let Some(epoch_event) = epoch_handler.tick(tick).await else {
                        continue;
                    };
                    handle_epoch_event(epoch_event, &mut crypto_processor, &mut backend);
                }
                Some(pol_info) = secret_pol_info_stream.next() => {
                    tracing::trace!(target: LOG_TARGET, "Received new winning slot info: {:?}", pol_info);
                }
                Some(session_event) = session_stream.next() => {
                    match handle_session_event(session_event, crypto_processor, &blend_config) {
                        Ok(new_crypto_processor) => crypto_processor = new_crypto_processor,
                        Err(e) => {
                            tracing::error!(
                                target: LOG_TARGET,
                                "Terminating the '{}' service as the new membership does not satisfy the core node condition: {e:?}",
                                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
                            );
                            return Err(e.into());
                        },
                    }
                }
            }
        }
    }
}

/// Handles a [`SessionEvent`].
///
/// It consumes the previous cryptographic processor and creates a new one
/// on a new session with its new membership.
/// It ignores the transition period expiration event and returns the previous
/// cryptographic processor as is.
fn handle_session_event<NodeId, ProofsGenerator, ProofsVerifier, BackendSettings>(
    event: SessionEvent<CoreSessionInfo<NodeId>>,
    cryptographic_processor: CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
    current_epoch_info: &LeaderInputsMinusQuota,
    settings: &BlendConfig<BackendSettings>,
) -> Result<CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>, Error>
where
    NodeId: Eq + Hash + Send,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
{
    match event {
        SessionEvent::NewSession(CoreSessionInfo {
            membership,
            poq_core_private_inputs,
            poq_core_public_inputs,
            session,
        }) => {
            let new_public_inputs = PoQVerificationInputsMinusSigningKey {
                core: poq_core_public_inputs,
                leader: LeaderInputs {
                    pol_ledger_aged: current_epoch_info.pol_ledger_aged,
                    pol_epoch_nonce: current_epoch_info.pol_epoch_nonce,
                    message_quota: settings.crypto.num_blend_layers,
                    total_stake: current_epoch_info.total_stake,
                },
                session,
            };
            Ok(
                CoreCryptographicProcessor::try_new_with_core_condition_check(
                    membership,
                    settings.minimum_network_size,
                    &settings.crypto,
                    new_public_inputs,
                    poq_core_private_inputs,
                )?,
            )
        }
        SessionEvent::TransitionPeriodExpired => Ok(cryptographic_processor),
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
    ProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
>(
    local_data_message: ServiceMessage<BroadcastSettings>,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    backend: &Backend,
    scheduler: &mut MessageScheduler<SessionClock, Rng, ProcessedMessage<BroadcastSettings>>,
) where
    NodeId: Eq + Hash + Send + 'static,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId> + Sync,
    BroadcastSettings: Serialize + for<'de> Deserialize<'de> + Send,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
{
    let ServiceMessage::Blend(message_payload) = local_data_message;

    let serialized_data_message = NetworkMessage::<BroadcastSettings>::to_bytes(&message_payload)
        .expect("NetworkMessage should be able to be serialized")
        .to_vec();

    let Ok(wrapped_message) = cryptographic_processor
        .encapsulate_data_payload(&serialized_data_message)
        .await
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
fn handle_incoming_blend_message<
    Rng,
    NodeId,
    SessionClock,
    BroadcastSettings,
    ProofsGenerator,
    ProofsVerifier,
>(
    validated_encapsulated_message: IncomingEncapsulatedMessageWithValidatedPublicHeader,
    scheduler: &mut MessageScheduler<SessionClock, Rng, ProcessedMessage<BroadcastSettings>>,
    cryptographic_processor: &CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
) where
    NodeId: 'static,
    BroadcastSettings: Serialize + for<'de> Deserialize<'de> + Send,
    ProofsVerifier: ProofsVerifierTrait,
{
    let Ok(decapsulated_message) = cryptographic_processor.decapsulate_message(validated_encapsulated_message).inspect_err(|e| {
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
                        NetworkMessage::from_bytes(&serialized_data_message)
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
async fn handle_release_round<
    NodeId,
    Rng,
    Backend,
    NetAdapter,
    ProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
>(
    RoundInfo {
        cover_message_generation_flag,
        processed_messages,
    }: RoundInfo<ProcessedMessage<NetAdapter::BroadcastSettings>>,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    rng: &mut Rng,
    backend: &Backend,
    network_adapter: &NetAdapter,
) where
    NodeId: Eq + Hash + 'static,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId> + Sync,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
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
            .await
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

fn handle_epoch_event<NodeId, ProofsGenerator, ProofsVerifier, Backend, Rng, RuntimeServiceId>(
    epoch_event: EpochEvent,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    backend: &mut Backend,
    settings: &BlendConfig<Backend::Settings>,
) where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    Backend: BlendBackend<NodeId, Rng, ProofsVerifier, RuntimeServiceId>,
{
    match epoch_event {
        EpochEvent::NewEpoch(new_epoch_public_info) => {
            let new_inputs = LeaderInputs {
                message_quota: settings.crypto.num_blend_layers,
                pol_epoch_nonce: new_epoch_public_info.pol_epoch_nonce,
                pol_ledger_aged: new_epoch_public_info.pol_ledger_aged,
                total_stake: new_epoch_public_info.total_stake,
            };
            cryptographic_processor.rotate_epoch(new_inputs);
            backend.rotate_epoch(new_inputs);
        }
        EpochEvent::OldEpochTransitionPeriodExpired => {
            cryptographic_processor.complete_epoch_transition();
            backend.complete_epoch_transition();
        }
        EpochEvent::NewEpochAndOldEpochTransitionExpired(new_epoch_public_info) => {
            let new_inputs = LeaderInputs {
                message_quota: settings.crypto.num_blend_layers,
                pol_epoch_nonce: new_epoch_public_info.pol_epoch_nonce,
                pol_ledger_aged: new_epoch_public_info.pol_ledger_aged,
                total_stake: new_epoch_public_info.total_stake,
            };
            cryptographic_processor.complete_epoch_transition();
            backend.complete_epoch_transition();
            cryptographic_processor.rotate_epoch(new_inputs);
            backend.rotate_epoch(new_inputs);
        }
    }
}

fn map_session_event_stream<InputStream, Input, Output, MappingFn>(
    input_stream: InputStream,
    mapping_fn: MappingFn,
) -> impl Stream<Item = SessionEvent<Output>>
where
    InputStream: Stream<Item = SessionEvent<Input>>,
    MappingFn: FnOnce(Input) -> Output + Copy + 'static,
{
    input_stream.map(move |event| match event {
        SessionEvent::NewSession(input) => SessionEvent::NewSession(mapping_fn(input)),
        SessionEvent::TransitionPeriodExpired => SessionEvent::TransitionPeriodExpired,
    })
}
