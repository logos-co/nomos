pub mod backends;
pub mod network;
mod processor;
pub mod settings;

use std::{hash::Hash, marker::PhantomData};

use async_trait::async_trait;
use backends::BlendBackend;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use futures::{
    StreamExt as _,
    future::{join_all, ready},
    stream::{BoxStream, repeat},
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
    session::SessionEvent,
};
use nomos_core::codec::{DeserializeOp as _, SerializeOp as _};
use nomos_time::SlotTick;
use nomos_utils::blake_rng::BlakeRng;
use rand::{RngCore, SeedableRng as _, seq::SliceRandom as _};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    context::{self, Context},
    core::{
        backends::{PublicInfo, SessionInfo},
        processor::{CoreCryptographicProcessor, Error},
        settings::BlendConfig,
    },
    epoch_info::{EpochEvent, EpochHandler, LeaderInputsMinusQuota},
    membership::{MembershipInfo, ZkInfo},
    message::{NetworkMessage, ProcessedMessage, ServiceMessage},
    mode::{self, Mode, ModeKind},
    session::{CoreSessionInfo, CoreSessionPublicInfo},
};

const LOG_TARGET: &str = "blend::service::core";

/// A core blend mode that sends messages to the blend network
/// and broadcasts fully unwrapped messages through the [`NetworkService`].
///
/// The blend backend and the network adapter are generic types that are
/// independent of each other. For example, the blend backend can use the
/// libp2p network stack, while the network adapter can use the other network
/// backend.
pub struct CoreMode<
    Backend,
    NodeId,
    Network,
    MembershipAdapter,
    ProofsGenerator,
    ProofsVerifier,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    NodeId: Clone,
    Network: NetworkAdapter<RuntimeServiceId>,
    ChainService: CryptarchiaServiceData,
{
    context: Context<NodeId, Network::BroadcastSettings, ChainService, RuntimeServiceId>,
    settings: BlendConfig<Backend::Settings>,
    backend: Backend,
    network_adapter: Network,
    incoming_message_stream:
        BoxStream<'static, IncomingEncapsulatedMessageWithValidatedPublicHeader>,
    crypto_processor: CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
    message_scheduler: MessageScheduler<
        BoxStream<'static, SchedulerSessionInfo>,
        BlakeRng,
        ProcessedMessage<Network::BroadcastSettings>,
    >,
    current_public_info: PublicInfo<NodeId>,
    blending_token_collector: BlendingTokenCollector,

    _phantom: PhantomData<(
        Backend,
        MembershipAdapter,
        ProofsGenerator,
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
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
>
    CoreMode<
        Backend,
        NodeId,
        Network,
        MembershipAdapter,
        ProofsGenerator,
        ProofsVerifier,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    Network: NetworkAdapter<RuntimeServiceId>,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync>,
    RuntimeServiceId: Clone + Send + Sync,
{
    pub fn new(
        context: Context<NodeId, Network::BroadcastSettings, ChainService, RuntimeServiceId>,
        settings: BlendConfig<Backend::Settings>,
        network_adapter: Network,
    ) -> Result<Self, &'static str> {
        let current_core_session_info =
            new_core_session_info(context.current_membership_info.clone(), &settings);
        let current_public_info = PublicInfo {
            epoch: LeaderInputs {
                pol_ledger_aged: context.current_leader_inputs_minus_quota.pol_ledger_aged,
                pol_epoch_nonce: context.current_leader_inputs_minus_quota.pol_epoch_nonce,
                message_quota: settings.crypto.num_blend_layers,
                total_stake: context.current_leader_inputs_minus_quota.total_stake,
            },
            session: SessionInfo {
                membership: current_core_session_info.public.membership.clone(),
                session_number: current_core_session_info.public.session,
                core_public_inputs: current_core_session_info.public.poq_core_public_inputs,
            },
        };

        let crypto_processor =
            CoreCryptographicProcessor::<_, ProofsGenerator, _>::try_new_with_core_condition_check(
                current_core_session_info.public.membership.clone(),
                settings.minimum_network_size,
                &settings.crypto,
                current_public_info.clone().into(),
                current_core_session_info.private,
            )
            .expect("The initial membership should satisfy the core node condition");

        let blending_token_collector = BlendingTokenCollector::new(
            &reward::SessionInfo::new(
                current_core_session_info.public.session,
                &current_public_info.epoch.pol_epoch_nonce,
                current_core_session_info.public.membership.size() as u64,
                current_core_session_info.public.poq_core_public_inputs.quota,
                settings.scheduler.cover.message_frequency_per_round,
            )
            .expect("Reward session info must be created successfully. Panicking since the service cannot continue with this session")
        );

        let message_scheduler = {
            let initial_scheduler_session_info = SchedulerSessionInfo {
                core_quota: settings
                    .session_quota(current_core_session_info.public.membership.size()),
                session_number: u128::from(current_core_session_info.public.session).into(),
            };
            let scheduler_stream = context
                .session_stream
                .clone()
                .zip(repeat(settings.clone()))
                .filter_map(|(event, settings)| {
                    ready(if let SessionEvent::NewSession(membership_info) = event {
                        Some(SchedulerSessionInfo {
                            core_quota: settings.session_quota(membership_info.membership.size()),
                            session_number: u128::from(membership_info.session_number).into(),
                        })
                    } else {
                        None
                    })
                });
            MessageScheduler::<_, _, ProcessedMessage<Network::BroadcastSettings>>::new(
                scheduler_stream.boxed(),
                initial_scheduler_session_info,
                BlakeRng::from_entropy(),
                settings.scheduler_settings(),
            )
        };

        let mut backend = Backend::new(
            settings.clone(),
            context.overwatch_handle.clone(),
            current_public_info.clone(),
            BlakeRng::from_entropy(),
        );
        let incoming_message_stream = backend.listen_to_incoming_messages();

        Ok(Self {
            context,
            settings,
            backend,
            network_adapter,
            incoming_message_stream,
            crypto_processor,
            message_scheduler,
            current_public_info,
            blending_token_collector,
            _phantom: PhantomData,
        })
    }
}

#[async_trait]
impl<
    Backend,
    NodeId,
    Network,
    MembershipAdapter,
    ProofsGenerator,
    ProofsVerifier,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> Mode<Context<NodeId, Network::BroadcastSettings, ChainService, RuntimeServiceId>>
    for CoreMode<
        Backend,
        NodeId,
        Network,
        MembershipAdapter,
        ProofsGenerator,
        ProofsVerifier,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend:
        BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId> + Send + Sync + 'static,
    NodeId: Eq + Hash + Clone + Send + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync + 'static,
    MembershipAdapter: Send,
    ProofsGenerator: CoreAndLeaderProofsGenerator + Send + 'static,
    ProofsVerifier: ProofsVerifierTrait + Send + 'static,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync> + Sync,
    PolInfoProvider: Send,
    RuntimeServiceId: Clone + Send + Sync + 'static,
{
    async fn run(
        mut self: Box<Self>,
    ) -> Result<
        (
            ModeKind,
            Context<NodeId, Network::BroadcastSettings, ChainService, RuntimeServiceId>,
        ),
        mode::Error,
    > {
        let Self {
            mut context,
            settings,
            mut backend,
            network_adapter,
            mut incoming_message_stream,
            mut crypto_processor,
            mut message_scheduler,
            mut current_public_info,
            mut blending_token_collector,
            ..
        } = *self;

        // Rng for releasing messages.
        let mut rng = BlakeRng::from_entropy();

        loop {
            tokio::select! {
                Some(incoming_message) = incoming_message_stream.next() => {
                    handle_incoming_blend_message(incoming_message, &mut message_scheduler, &crypto_processor, &mut blending_token_collector);
                }
                Some(round_info) = message_scheduler.next() => {
                    handle_release_round(round_info, &mut crypto_processor, &mut rng, &backend, &network_adapter).await;
                }
                Some(event) = context.advance() => {
                    match event {
                        context::Event::ServiceMessage(message) => {
                            handle_local_data_message(message, &mut crypto_processor, &backend, &mut message_scheduler).await;
                        },
                        context::Event::ClockTick(clock_tick) => {
                            (current_public_info, context.current_leader_inputs_minus_quota) = handle_clock_event(clock_tick, &settings, &mut context.epoch_handler, &mut crypto_processor, &mut backend, current_public_info, context.current_leader_inputs_minus_quota).await;
                        },
                        context::Event::SecretPolInfo(pol_info) => {
                            handle_new_secret_epoch_info(pol_info.poq_private_inputs, &mut crypto_processor);
                        },
                        context::Event::SessionEvent(session_event) => {
                            match handle_session_event(session_event, &settings, crypto_processor, current_public_info, &mut blending_token_collector, &mut backend).await {
                                Ok((new_crypto_processor, new_public_info)) => {
                                    crypto_processor = new_crypto_processor;
                                    current_public_info = new_public_info;
                                }
                                Err((Error::LocalIsNotCoreNode, prev_crypto_processor, new_membership_info)) => {
                                    tracing::info!(
                                        target: LOG_TARGET,
                                        "Terminating core mode as the node in the new membership is not core",
                                    );
                                    // Spawn-and-forget the final session transition handler
                                    let final_session_transition_handler = FinalSessionTransitionTask {
                                        settings,
                                        backend,
                                        network_adapter,
                                        incoming_message_stream,
                                        message_scheduler,
                                        crypto_processor: prev_crypto_processor,
                                        blending_token_collector,
                                    };
                                    context.overwatch_handle.runtime().spawn(async move {
                                        final_session_transition_handler.run().await;
                                    });

                                    context.current_membership_info = new_membership_info;
                                    return Ok((ModeKind::Edge, context));
                                }
                                Err((Error::NetworkIsTooSmall(size), _, new_membership_info)) => {
                                    tracing::info!(
                                        target: LOG_TARGET,
                                        "Terminating core mode as the new membership is too small: {size}",
                                    );
                                    context.current_membership_info = new_membership_info;
                                    return Ok((ModeKind::Broadcast, context));
                                }
                            }
                        },
                    }
                }
            }
        }
    }
}

/// A task that finishes the final session transition period when the node is
/// transitioning to a mode other than core.
///
/// This task is not needed when the node remains in core mode because the
/// resources (e.g. backend) must be shared for both the current and the
/// previous sessions. But, when transiting to other modes, this task should be
/// spawned to handle any remaining messages in the previous session until the
/// session transition period ends.
struct FinalSessionTransitionTask<
    Backend,
    NodeId,
    Network,
    ProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
    Network: NetworkAdapter<RuntimeServiceId>,
{
    settings: BlendConfig<Backend::Settings>,
    backend: Backend,
    network_adapter: Network,
    incoming_message_stream:
        BoxStream<'static, IncomingEncapsulatedMessageWithValidatedPublicHeader>,
    message_scheduler: MessageScheduler<
        BoxStream<'static, SchedulerSessionInfo>,
        BlakeRng,
        ProcessedMessage<Network::BroadcastSettings>,
    >,
    crypto_processor: CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
    blending_token_collector: BlendingTokenCollector,
}

impl<Backend, NodeId, Network, ProofsGenerator, ProofsVerifier, RuntimeServiceId>
    FinalSessionTransitionTask<
        Backend,
        NodeId,
        Network,
        ProofsGenerator,
        ProofsVerifier,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId> + Send + Sync,
    NodeId: Eq + Hash + Clone + Send + 'static,
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync,
    ProofsGenerator: CoreAndLeaderProofsGenerator + Send,
    ProofsVerifier: ProofsVerifierTrait + Send,
    RuntimeServiceId: Send + Sync,
{
    async fn run(self) {
        let Self {
            settings,
            backend,
            network_adapter,
            mut incoming_message_stream,
            mut message_scheduler,
            mut crypto_processor,
            mut blending_token_collector,
        } = self;

        let mut session_transition_period_timer = Box::pin(tokio::time::sleep(
            settings.time.session_transition_period(),
        ));

        // Rng for releasing messages.
        let mut rng = BlakeRng::from_entropy();

        loop {
            tokio::select! {
                () = &mut session_transition_period_timer => {
                    info!(target: LOG_TARGET, "Final session transition period has passed");
                    return;
                }
                Some(incoming_message) = incoming_message_stream.next() => {
                    handle_incoming_blend_message(incoming_message, &mut message_scheduler, &crypto_processor, &mut blending_token_collector);
                }
                Some(round_info) = message_scheduler.next() => {
                    handle_release_round(round_info, &mut crypto_processor, &mut rng, &backend, &network_adapter).await;
                }
            }
        }
    }
}

/// Builds a [`CoreSessionInfo`] from a [`MembershipInfo`].
fn new_core_session_info<NodeId, BackendSettings>(
    MembershipInfo {
        membership,
        zk: ZkInfo {
            core_and_path_selectors,
            root: zk_root,
        },
        session_number,
    }: MembershipInfo<NodeId>,
    settings: &BlendConfig<BackendSettings>,
) -> CoreSessionInfo<NodeId> {
    let membership_size = membership.size();
    CoreSessionInfo {
        public: CoreSessionPublicInfo {
            membership,
            session: session_number,
            poq_core_public_inputs: CoreInputs {
                quota: settings.session_quota(membership_size),
                zk_root,
            },
        },
        private: ProofOfCoreQuotaInputs {
            core_sk: *settings.zk.sk.as_fr(),
            core_path_and_selectors: core_and_path_selectors
                .expect("Core merkle path should be present for a core node."),
        },
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
    event: SessionEvent<MembershipInfo<NodeId>>,
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
    (
        Error,
        CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
        MembershipInfo<NodeId>,
    ),
>
where
    NodeId: Eq + Hash + Clone + Send,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    Backend: BlendBackend<NodeId, BlakeRng, ProofsVerifier, RuntimeServiceId>,
{
    match event {
        SessionEvent::NewSession(new_membership_info) => {
            let CoreSessionInfo {
                public:
                    CoreSessionPublicInfo {
                        membership,
                        session: session_number,
                        poq_core_public_inputs,
                    },
                private,
            } = new_core_session_info(new_membership_info.clone(), settings);

            blending_token_collector.rotate_session(&reward::SessionInfo::new(
                session_number,
                &current_public_info.epoch.pol_epoch_nonce,
                membership.size() as u64,
                poq_core_public_inputs.quota,
                settings.scheduler.cover.message_frequency_per_round,
            )
            .expect("Reward session info must be created successfully. Panicking since the service cannot continue with this session"));

            let new_session_info = SessionInfo {
                membership: membership.clone(),
                session_number,
                core_public_inputs: poq_core_public_inputs,
            };
            backend.rotate_session(new_session_info.clone()).await;

            let new_public_info = PublicInfo {
                session: new_session_info,
                ..current_public_info
            };
            let new_processor = CoreCryptographicProcessor::try_new_with_core_condition_check(
                membership,
                settings.minimum_network_size,
                &settings.crypto,
                new_public_info.clone().into(),
                private,
            )
            .map_err(|e| (e, current_cryptographic_processor, new_membership_info))?;
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
    epoch_handler: &mut EpochHandler<
        CryptarchiaServiceApi<ChainService, RuntimeServiceId>,
        RuntimeServiceId,
    >,
    cryptographic_processor: &mut CoreCryptographicProcessor<
        NodeId,
        ProofsGenerator,
        ProofsVerifier,
    >,
    backend: &mut Backend,
    current_public_info: PublicInfo<NodeId>,
    current_leader_inputs_minus_quota: LeaderInputsMinusQuota,
) -> (PublicInfo<NodeId>, LeaderInputsMinusQuota)
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync> + Sync,
    Backend: BlendBackend<NodeId, Rng, ProofsVerifier, RuntimeServiceId>,
    RuntimeServiceId: Send + Sync,
{
    let Some(epoch_event) = epoch_handler.tick(slot_tick).await else {
        return (current_public_info, current_leader_inputs_minus_quota);
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

            (
                new_public_info,
                LeaderInputsMinusQuota {
                    pol_ledger_aged,
                    pol_epoch_nonce,
                    total_stake,
                },
            )
        }
        EpochEvent::OldEpochTransitionPeriodExpired => {
            cryptographic_processor.complete_epoch_transition();
            backend.complete_epoch_transition().await;

            (current_public_info, current_leader_inputs_minus_quota)
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

            (
                new_public_inputs,
                LeaderInputsMinusQuota {
                    pol_ledger_aged,
                    pol_epoch_nonce,
                    total_stake,
                },
            )
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
