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
use futures::{
    StreamExt as _,
    future::{join_all, ready},
};
use network::NetworkAdapter;
use nomos_blend_message::{
    PayloadType,
    crypto::{
        proofs::quota::inputs::prove::{
            private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
            public::{CoreInputs, LeaderInputs},
        },
        random_sized_bytes,
    },
    encap::{ProofsVerifier as ProofsVerifierTrait, decapsulated::DecapsulationOutput},
    reward::{self, ActivityProof, BlendingTokenCollector},
};
use nomos_blend_scheduling::{
    message_blend::{
        crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
        provers::core_and_leader::CoreAndLeaderProofsGenerator,
    },
    message_scheduler::{
        MessageScheduler, round_info::RoundInfo, session_info::SessionInfo as SchedulerSessionInfo,
    },
    session::{SessionEvent, UninitializedSessionEventStream},
    stream::UninitializedFirstReadyStream,
};
use nomos_core::codec::{DeserializeOp as _, SerializeOp as _};
use nomos_network::NetworkService;
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, state::StateUpdater},
};
use rand::{RngCore, SeedableRng as _, seq::SliceRandom as _};
use serde::{Deserialize, Serialize};
use services_utils::{overwatch::RecoveryOperator, wait_until_services_are_ready};
use tokio::sync::oneshot;
use tracing::info;

use crate::{
    core::{
        backends::{PublicInfo, SessionInfo},
        processor::{CoreCryptographicProcessor, Error},
        settings::BlendConfig,
        state::{RecoveryServiceState, RoundInfoAndConsumedQuota, SchedulerWrapper, ServiceState},
    },
    epoch_info::{
        ChainApi, EpochEvent, EpochHandler, LeaderInputsMinusQuota,
        PolInfoProvider as PolInfoProviderTrait,
    },
    membership::{self, MembershipInfo, ZkInfo},
    message::{NetworkMessage, ProcessedMessage, ServiceMessage},
    session::{CoreSessionInfo, CoreSessionPublicInfo},
    settings::FIRST_STREAM_ITEM_READY_TIMEOUT,
};

pub mod backends;
pub mod network;
pub mod settings;

pub(super) mod service_components;

mod processor;
mod state;
pub use state::RecoveryServiceState as CoreServiceState;

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
    StateRecoveryBackend,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    last_saved_state: Option<ServiceState>,
    _phantom: PhantomData<(
        Backend,
        MembershipAdapter,
        ProofsGenerator,
        TimeBackend,
        ChainService,
        StateRecoveryBackend,
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
    StateRecoveryBackend,
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
        StateRecoveryBackend,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    type Settings = BlendConfig<Backend::Settings>;
    type State = RecoveryServiceState<Backend::Settings>;
    type StateOperator = RecoveryOperator<StateRecoveryBackend>;
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
    StateRecoveryBackend,
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
        StateRecoveryBackend,
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
    StateRecoveryBackend: Send,
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
        recovery_initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            service_resources_handle,
            last_saved_state: recovery_initial_state.service_state,
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
                    ref state_updater,
                },
            last_saved_state,
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
            let network_relay = overwatch_handle
                .relay::<NetworkService<_, _>>()
                .await
                .expect("Relay with network service should be available.");
            Network::new(network_relay)
        }
        .await;

        let mut epoch_handler = async {
            let chain_service = CryptarchiaServiceApi::<ChainService, _>::new(
                overwatch_handle
                    .relay::<ChainService>()
                    .await
                    .expect("Failed to establish channel with chain service."),
            );
            EpochHandler::new(
                chain_service,
                blend_config.time.epoch_transition_period_in_slots,
            )
        }
        .await;

        // Initialize membership stream for session and core-related public PoQ inputs.
        let session_stream = async {
            let config = blend_config.clone();
            let zk_secret_key = config.zk.sk.clone();

            MembershipAdapter::new(
                overwatch_handle
                    .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                    .await
                    .expect("Failed to get relay channel with membership service."),
                blend_config.crypto.non_ephemeral_signing_key.public_key(),
                Some(blend_config.zk.public_key()),
            )
            .subscribe()
            .await
            .expect("Failed to get membership stream from membership service.")
            .map(
                move |MembershipInfo {
                          membership,
                          session_number,
                          zk:
                              ZkInfo {
                                  core_and_path_selectors,
                                  root: zk_root,
                              },
                      }| CoreSessionInfo {
                    public: CoreSessionPublicInfo {
                        poq_core_public_inputs: CoreInputs {
                            quota: config.session_quota(membership.size()),
                            zk_root,
                        },
                        membership,
                        session: session_number,
                    },
                    private: ProofOfCoreQuotaInputs {
                        core_sk: *zk_secret_key.as_fr(),
                        core_path_and_selectors: core_and_path_selectors
                            .expect("Core merkle path should be present for a core node."),
                    },
                },
            )
        }
        .await;

        // Initialize clock stream for epoch-related public PoQ inputs.
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

        let (current_membership_info, mut remaining_session_stream) = Box::pin(
            UninitializedSessionEventStream::new(
                session_stream,
                FIRST_STREAM_ITEM_READY_TIMEOUT,
                blend_config.time.session_transition_period(),
            )
            .await_first_ready(),
        )
        .await
        .map(|(membership_info, remaining_session_stream)| {
            (membership_info, remaining_session_stream.fork())
        })
        .expect("The current session info must be available.");

        let (
            LeaderInputsMinusQuota {
                pol_epoch_nonce,
                pol_ledger_aged,
                total_stake,
            },
            mut remaining_clock_stream,
        ) = async {
            let (clock_tick, remaining_clock_stream) =
                UninitializedFirstReadyStream::new(clock_stream, Duration::from_secs(5))
                    .first()
                    .await
                    .expect("The clock system must be available.");
            let Some(EpochEvent::NewEpoch(new_epoch_info)) = epoch_handler.tick(clock_tick).await
            else {
                panic!("First poll result of epoch stream should be a `NewEpoch` event.");
            };
            (new_epoch_info, remaining_clock_stream)
        }
        .await;

        info!(
            target: LOG_TARGET,
            "The current membership is ready: {} nodes.",
            current_membership_info.public.membership.size()
        );

        let mut current_public_info = PublicInfo {
            epoch: LeaderInputs {
                pol_ledger_aged,
                pol_epoch_nonce,
                message_quota: blend_config.crypto.num_blend_layers,
                total_stake,
            },
            session: SessionInfo {
                membership: current_membership_info.public.membership.clone(),
                session_number: current_membership_info.public.session,
                core_public_inputs: current_membership_info.public.poq_core_public_inputs,
            },
        };

        let mut crypto_processor =
            CoreCryptographicProcessor::<_, ProofsGenerator, _>::try_new_with_core_condition_check(
                current_membership_info.public.membership.clone(),
                blend_config.minimum_network_size,
                &blend_config.crypto,
                current_public_info.clone().into(),
                current_membership_info.private,
            )
            .expect("The initial membership should satisfy the core node condition");

        let mut blending_token_collector = BlendingTokenCollector::new(
            &reward::SessionInfo::new(
                current_membership_info.public.session,
                &pol_epoch_nonce,
                current_membership_info.public.membership.size() as u64,
                current_membership_info.public.poq_core_public_inputs.quota,
                blend_config.scheduler.cover.message_frequency_per_round,
            )
            .expect("Reward session info must be created successfully. Panicking since the service cannot continue with this session")
        );

        // Initialize the current session state. If the session matches the stored one,
        // retrieves the tracked consumed core quota. Else, fallback to `0`.
        let mut current_session_state = if let Some(saved_state) = last_saved_state
            && saved_state.last_seen_session() == current_membership_info.public.session
        {
            saved_state
        } else {
            ServiceState::new(current_membership_info.public.session, 0)
        };

        let mut message_scheduler = {
            let scheduler_starting_core_quota = blend_config
                .session_quota(current_membership_info.public.membership.size())
                .saturating_sub(current_session_state.spent_core_quota());
            let initial_scheduler_session_info = SchedulerSessionInfo {
                core_quota: scheduler_starting_core_quota,
                session_number: u128::from(current_membership_info.public.session).into(),
            };
            let scheduler_stream = remaining_session_stream
                .clone()
                .filter_map(|event| {
                    ready(if let SessionEvent::NewSession(new_session_info) = event {
                        Some(new_session_info)
                    } else {
                        None
                    })
                })
                .map(
                    |CoreSessionInfo {
                         public:
                             CoreSessionPublicInfo {
                                 membership,
                                 session,
                                 ..
                             },
                         ..
                     }| SchedulerSessionInfo {
                        core_quota: blend_config.session_quota(membership.size()),
                        session_number: u128::from(session).into(),
                    },
                );
            SchedulerWrapper::<_, _, ProcessedMessage<Network::BroadcastSettings>>::new(
                scheduler_stream,
                initial_scheduler_session_info,
                BlakeRng::from_entropy(),
                blend_config.scheduler_settings(),
            )
        };

        let mut backend = Backend::new(
            blend_config.clone(),
            overwatch_handle.clone(),
            current_public_info.clone(),
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
        let mut secret_pol_info_stream = PolInfoProvider::subscribe(overwatch_handle)
            .await
            .expect("Should not fail to subscribe to secret PoL info stream.");

        loop {
            tokio::select! {
                Some(local_data_message) = inbound_relay.next() => {
                    handle_local_data_message(local_data_message, &mut crypto_processor, &backend, &mut message_scheduler).await;
                }
                Some(incoming_message) = blend_messages.next() => {
                    handle_incoming_blend_message(incoming_message, &mut message_scheduler, &crypto_processor, &mut blending_token_collector);
                }
                Some(round_info) = message_scheduler.next() => {
                    current_session_state = handle_release_round(round_info, &mut crypto_processor, &mut rng, &backend, &network_adapter, state_updater, current_session_state).await;
                }
                Some(clock_tick) = remaining_clock_stream.next() => {
                    current_public_info = handle_clock_event(clock_tick, &blend_config, &mut epoch_handler, &mut crypto_processor, &mut backend, current_public_info).await;
                }
                Some(pol_info) = secret_pol_info_stream.next() => {
                    handle_new_secret_epoch_info(pol_info.poq_private_inputs, &mut crypto_processor);
                }
                Some(session_event) = remaining_session_stream.next() => {
                    match handle_session_event(session_event, &blend_config, crypto_processor, current_public_info, &mut blending_token_collector, &mut backend).await {
                        Ok((new_crypto_processor, new_public_info)) => {
                            crypto_processor = new_crypto_processor;
                            current_public_info = new_public_info;
                        }
                        Err(e) => {
                            tracing::error!(
                                target: LOG_TARGET,
                                "Terminating the '{}' service as the new membership does not satisfy the core node condition: {e:?}",
                                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
                            );
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    }
}

/// Handles a [`SessionEvent`].
///
/// It consumes the previous cryptographic processor and creates a new one
/// on a new session with its new membership. It also creates new public inputs
/// for `PoQ` verification in this new session. It ignores the transition period
/// expiration event and returns the previous cryptographic processor as is.
async fn handle_session_event<
    NodeId,
    ProofsGenerator,
    ProofsVerifier,
    Backend,
    BlakeRng,
    RuntimeServiceId,
>(
    event: SessionEvent<CoreSessionInfo<NodeId>>,
    settings: &BlendConfig<Backend::Settings>,
    current_cryptographic_processor: CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    current_public_info: PublicInfo<NodeId>,
    blending_token_collector: &mut BlendingTokenCollector,
    backend: &mut Backend,
) -> Result<
    (
        CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
        PublicInfo<NodeId>,
    ),
    Error,
>
where
    NodeId: Eq + Hash + Clone + Send,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
{
    match event {
        SessionEvent::NewSession(CoreSessionInfo {
            private,
            public:
                CoreSessionPublicInfo {
                    poq_core_public_inputs: new_core_public_inputs,
                    session: new_session,
                    membership: new_membership,
                },
        }) => {
            blending_token_collector.rotate_session(&reward::SessionInfo::new(
                new_session,
                &current_public_info.epoch.pol_epoch_nonce,
                new_membership.size() as u64,
                new_core_public_inputs.quota,
                settings.scheduler.cover.message_frequency_per_round,
            )
            .expect("Reward session info must be created successfully. Panicking since the service cannot continue with this session"));

            let new_session_info = SessionInfo {
                membership: new_membership.clone(),
                session_number: new_session,
                core_public_inputs: new_core_public_inputs,
            };
            backend.rotate_session(new_session_info.clone()).await;

            let new_public_info = PublicInfo {
                session: new_session_info,
                ..current_public_info
            };
            let new_processor = CoreCryptographicProcessor::try_new_with_core_condition_check(
                new_membership,
                settings.minimum_network_size,
                &settings.crypto,
                new_public_info.clone().into(),
                private,
            )?;
            Ok((new_processor, new_public_info))
        }
        SessionEvent::TransitionPeriodExpired => {
            if let Some(activity_proof) =
                blending_token_collector.compute_activity_proof_for_previous_session()
            {
                info!(target: LOG_TARGET, "Activity proof generated: {activity_proof:?}");
                submit_activity_proof(activity_proof);
            }
            backend.complete_session_transition().await;
            Ok((current_cryptographic_processor, current_public_info))
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
    scheduler: &mut SchedulerWrapper<SessionClock, Rng, ProcessedMessage<BroadcastSettings>>,
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
    blending_token_collector: &mut BlendingTokenCollector,
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
        DecapsulationOutput::Completed {
            fully_decapsulated_message,
            blending_token,
        } => {
            blending_token_collector.insert(blending_token);
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
        DecapsulationOutput::Incompleted {
            remaining_encapsulated_message,
            blending_token,
        } => {
            blending_token_collector.insert(blending_token);
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
    RoundInfoAndConsumedQuota {
        info:
            RoundInfo {
                cover_message_generation_flag,
                processed_messages,
            },
        consumed_quota,
    }: RoundInfoAndConsumedQuota<ProcessedMessage<NetAdapter::BroadcastSettings>>,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    rng: &mut Rng,
    backend: &Backend,
    network_adapter: &NetAdapter,
    state_updater: &StateUpdater<Option<RecoveryServiceState<Backend::Settings>>>,
    current_state: ServiceState,
) -> ServiceState
where
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

    // Update state by tracking the quota consumed by this release round.
    let new_state = current_state.with_consumed_core_quota(consumed_quota);
    state_updater.update(Some(new_state.clone().into()));
    new_state
}

/// Handle a clock event by calling into the epoch handler and process the
/// resulting epoch event, if any.
///
/// On a new epoch, it will update the cryptographic processor and the current
/// `PoQ` public inputs. At the end of an epoch transition period, it will
/// interact with the Blend components to communicate the end of such transition
/// period.
async fn handle_clock_event<
    NodeId,
    ProofsGenerator,
    ProofsVerifier,
    ChainService,
    Backend,
    Rng,
    RuntimeServiceId,
>(
    slot_tick: SlotTick,
    settings: &BlendConfig<Backend::Settings>,
    epoch_handler: &mut EpochHandler<ChainService, RuntimeServiceId>,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    backend: &mut Backend,
    current_public_info: PublicInfo<NodeId>,
) -> PublicInfo<NodeId>
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    ChainService: ChainApi<RuntimeServiceId> + Sync,
    Backend: BlendBackend<NodeId, Rng, ProofsVerifier, RuntimeServiceId>,
    RuntimeServiceId: Sync,
{
    let Some(epoch_event) = epoch_handler.tick(slot_tick).await else {
        return current_public_info;
    };

    match epoch_event {
        EpochEvent::NewEpoch(LeaderInputsMinusQuota {
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        }) => {
            let new_leader_inputs = LeaderInputs {
                message_quota: settings.crypto.num_blend_layers,
                pol_epoch_nonce,
                pol_ledger_aged,
                total_stake,
            };
            let new_public_info = PublicInfo {
                epoch: new_leader_inputs,
                ..current_public_info
            };

            cryptographic_processor.rotate_epoch(new_leader_inputs);
            backend.rotate_epoch(new_leader_inputs).await;

            new_public_info
        }
        EpochEvent::OldEpochTransitionPeriodExpired => {
            cryptographic_processor.complete_epoch_transition();
            backend.complete_epoch_transition().await;

            current_public_info
        }
        EpochEvent::NewEpochAndOldEpochTransitionExpired(LeaderInputsMinusQuota {
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        }) => {
            let new_leader_inputs = LeaderInputs {
                message_quota: settings.crypto.num_blend_layers,
                pol_epoch_nonce,
                pol_ledger_aged,
                total_stake,
            };
            let new_public_inputs = PublicInfo {
                epoch: new_leader_inputs,
                ..current_public_info
            };

            // Complete transition of previous epoch, then set the current epoch as the old
            // one and move to the new one.
            cryptographic_processor.complete_epoch_transition();
            backend.complete_epoch_transition().await;
            cryptographic_processor.rotate_epoch(new_leader_inputs);
            backend.rotate_epoch(new_leader_inputs).await;

            new_public_inputs
        }
    }
}

/// Handle the availability of new secret `PoL` info by passing it to the
/// underlying cryptographic processor.
fn handle_new_secret_epoch_info<NodeId, ProofsGenerator, ProofsVerifier>(
    new_pol_info: ProofOfLeadershipQuotaInputs,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
) where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
{
    cryptographic_processor.set_epoch_private(new_pol_info);
}

/// Submits an activity proof to the SDP service.
fn submit_activity_proof(_proof: ActivityProof) {
    todo!()
}
