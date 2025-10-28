pub mod backends;
mod handlers;
pub mod settings;
#[cfg(test)]
mod tests;

use std::{hash::Hash, marker::PhantomData};

use backends::BlendBackend;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use nomos_blend_message::crypto::proofs::{
    PoQVerificationInputsMinusSigningKey,
    quota::inputs::prove::{
        private::ProofOfLeadershipQuotaInputs,
        public::{CoreInputs, LeaderInputs},
    },
};
use nomos_blend_scheduling::{
    membership::Membership, message_blend::provers::leader::LeaderProofsGenerator,
    session::SessionEvent,
};
use nomos_core::codec::SerializeOp as _;
use nomos_time::SlotTick;
use overwatch::overwatch::OverwatchHandle;
use serde::Serialize;
use settings::BlendConfig;
use tracing::{debug, info};

use crate::{
    context::{self, Context},
    edge::handlers::{Error, MessageHandler},
    epoch_info::{EpochEvent, EpochHandler, LeaderInputsMinusQuota, PolEpochInfo},
    membership::{MembershipInfo, ZkInfo},
    message::{NetworkMessage, ServiceMessage},
    mode::{self, Mode, ModeKind},
};

const LOG_TARGET: &str = "blend::service::edge";

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings>;

pub struct EdgeMode<
    Backend,
    NodeId,
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
    ChainService: CryptarchiaServiceData,
{
    context: Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>,
    settings: BlendConfig<Backend::Settings>,
    message_handler: MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
    current_public_inputs: PoQVerificationInputsMinusSigningKey,
    _phantom: PhantomData<(MembershipAdapter, PolInfoProvider)>,
}

impl<
    Backend,
    NodeId,
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
>
    EdgeMode<
        Backend,
        NodeId,
        BroadcastSettings,
        MembershipAdapter,
        ProofsGenerator,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Send + Eq + Hash + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    ChainService: CryptarchiaServiceData + Sync,
    RuntimeServiceId: Clone + Sync,
{
    pub fn new(
        context: Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>,
        settings: BlendConfig<Backend::Settings>,
    ) -> Result<Self, &'static str> {
        let current_epoch_info = LeaderInputs {
            message_quota: settings.crypto.num_blend_layers,
            pol_ledger_aged: context.current_leader_inputs_minus_quota.pol_ledger_aged,
            pol_epoch_nonce: context.current_leader_inputs_minus_quota.pol_epoch_nonce,
            total_stake: context.current_leader_inputs_minus_quota.total_stake,
        };
        let current_public_inputs = PoQVerificationInputsMinusSigningKey {
            core: CoreInputs {
                zk_root: context.current_membership_info.zk.root,
                quota: settings.cover.session_quota(
                    &settings.crypto,
                    &settings.time,
                    context.current_membership_info.membership.size(),
                ),
            },
            leader: current_epoch_info,
            session: context.current_membership_info.session_number,
        };
        let message_handler =
            MessageHandler::<Backend, _, ProofsGenerator, _>::try_new_with_edge_condition_check(
                &settings,
                context.current_membership_info.membership.clone(),
                current_public_inputs,
                context
                    .current_private_leader_info
                    .poq_private_inputs
                    .clone(),
                context.overwatch_handle.clone(),
            )
            .expect("The initial membership should satisfy the edge node condition");

        Ok(Self {
            context,
            settings,
            message_handler,
            current_public_inputs,
            _phantom: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<
    Backend,
    NodeId,
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> Mode<Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>>
    for EdgeMode<
        Backend,
        NodeId,
        BroadcastSettings,
        MembershipAdapter,
        ProofsGenerator,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + Send,
    MembershipAdapter: Send,
    ProofsGenerator: LeaderProofsGenerator + Send,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync> + Send + Sync,
    PolInfoProvider: Send,
    RuntimeServiceId: Clone + Send + Sync,
{
    async fn run(
        mut self: Box<Self>,
    ) -> Result<
        (
            ModeKind,
            Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>,
        ),
        mode::Error,
    > {
        let Self {
            mut context,
            settings,
            mut message_handler,
            mut current_public_inputs,
            ..
        } = *self;

        while let Some(event) = context.advance().await {
            match event {
                context::Event::SessionEvent(SessionEvent::NewSession(new_membership_info)) => {
                    match handle_new_session(
                        new_membership_info,
                        &settings,
                        context
                            .current_private_leader_info
                            .poq_private_inputs
                            .clone(),
                        context.overwatch_handle.clone(),
                        current_public_inputs,
                        message_handler,
                    ) {
                        Ok((new_message_handler, new_public_inputs)) => {
                            message_handler = new_message_handler;
                            current_public_inputs = new_public_inputs;
                        }
                        Err(Error::LocalIsCoreNode) => {
                            info!(
                                target: LOG_TARGET,
                                "Terminating edge mode as the node in the new membership is core",
                            );
                            return Ok((ModeKind::Core, context));
                        }
                        Err(Error::NetworkIsTooSmall(size)) => {
                            info!(
                                target: LOG_TARGET,
                                "Terminating edge mode as the new membership is too small: {size}",
                            );
                            return Ok((ModeKind::Broadcast, context));
                        }
                    }
                }
                context::Event::SessionEvent(_) => {
                    // ignore other session events
                }
                context::Event::ServiceMessage(ServiceMessage::Blend(message)) => {
                    let message = NetworkMessage::<BroadcastSettings>::to_bytes(&message)
                        .expect("NetworkMessage should be able to be serialized")
                        .to_vec();
                    message_handler.handle_messages_to_blend(message).await;
                }
                context::Event::ClockTick(clock_tick) => {
                    let (new_message_handler, new_public_inputs) = handle_clock_event(
                        clock_tick,
                        &settings,
                        &context.current_private_leader_info,
                        &context.overwatch_handle,
                        &context.current_membership_info.membership,
                        &mut context.epoch_handler,
                        message_handler,
                        current_public_inputs,
                    )
                    .await;
                    message_handler = new_message_handler;
                    current_public_inputs = new_public_inputs;
                }
                context::Event::SecretPolInfo(new_secret_pol_info) => {
                    message_handler = handle_new_secret_epoch_info(
                        &new_secret_pol_info,
                        &settings,
                        &current_public_inputs,
                        &context.overwatch_handle,
                        &context.current_membership_info.membership,
                        message_handler,
                    );
                }
            }
        }
        unreachable!("Context shouldn't end");
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] and new `PoQ` public inputs if the
/// membership satisfies all the edge node condition. Otherwise, it returns
/// [`Error`].
#[expect(
    clippy::type_complexity,
    reason = "There are too many generics. Any type alias would be as complicated."
)]
fn handle_new_session<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    MembershipInfo {
        membership: new_membership,
        session_number: new_session_number,
        zk: ZkInfo {
            root: new_zk_root, ..
        },
    }: MembershipInfo<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_epoch_private_info: ProofOfLeadershipQuotaInputs,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    current_public_inputs: PoQVerificationInputsMinusSigningKey,
    // Unused, but we want to consume it.
    _current_message_handler: MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
) -> Result<
    (
        MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
        PoQVerificationInputsMinusSigningKey,
    ),
    Error,
>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    RuntimeServiceId: Clone,
{
    debug!(target: LOG_TARGET, "Trying to create a new message handler");
    // Update current public inputs with new session info.
    let new_public_inputs = PoQVerificationInputsMinusSigningKey {
        session: new_session_number,
        core: CoreInputs {
            quota: settings.cover.session_quota(
                &settings.crypto,
                &settings.time,
                new_membership.size(),
            ),
            zk_root: new_zk_root,
        },
        ..current_public_inputs
    };

    let new_handler = MessageHandler::try_new_with_edge_condition_check(
        settings,
        new_membership,
        new_public_inputs,
        current_epoch_private_info,
        overwatch_handle,
    )?;

    Ok((new_handler, new_public_inputs))
}

#[expect(
    clippy::too_many_arguments,
    reason = "TODO: Address this at some point."
)]
/// It fetches the new epoch state if a new epoch tick is provided, which will
/// result in new `PoQ` public inputs. A new handler is also created if the
/// secret `PoL` info for the same epoch has already been provided by the
/// `PoLInfoProvider`.
///
/// If the slot tick does not belong to a new, unprocessed epoch, the old
/// handler and inputs are returned instead.
/// In case this is called before the secret info is obtained, only the public
/// inputs are updated, and the message handler will be updated once the secret
/// info is received.
async fn handle_clock_event<Backend, NodeId, ProofsGenerator, ChainService, RuntimeServiceId>(
    slot_tick: SlotTick,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    PolEpochInfo {
        nonce: current_secret_inputs_nonce,
        poq_private_inputs: current_secret_inputs,
    }: &PolEpochInfo,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    current_membership: &Membership<NodeId>,
    epoch_handler: &mut EpochHandler<
        CryptarchiaServiceApi<ChainService, RuntimeServiceId>,
        RuntimeServiceId,
    >,
    current_message_handler: MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
    current_public_inputs: PoQVerificationInputsMinusSigningKey,
) -> (
    MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
    PoQVerificationInputsMinusSigningKey,
)
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    ProofsGenerator: LeaderProofsGenerator + Send,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync> + Send + Sync,
    RuntimeServiceId: Clone + Send + Sync,
{
    let Some(epoch_event) = epoch_handler.tick(slot_tick).await else {
        return (current_message_handler, current_public_inputs);
    };

    let new_leader_inputs = match epoch_event {
        EpochEvent::NewEpoch(LeaderInputsMinusQuota {
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        })
        | EpochEvent::NewEpochAndOldEpochTransitionExpired(LeaderInputsMinusQuota {
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        }) => LeaderInputs {
            message_quota: settings.crypto.num_blend_layers,
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        },
        // We don't handle the epoch transitions in edge node.
        EpochEvent::OldEpochTransitionPeriodExpired => current_public_inputs.leader,
    };

    let new_public_inputs = PoQVerificationInputsMinusSigningKey {
        leader: new_leader_inputs,
        ..current_public_inputs
    };

    // If the private info for the new epoch came in before, create a new message
    // handler with the new info.
    let new_message_handler = if new_public_inputs.leader.pol_epoch_nonce
        == *current_secret_inputs_nonce
    {
        MessageHandler::try_new_with_edge_condition_check(
            settings,
            current_membership.clone(),
            new_public_inputs,
            current_secret_inputs.clone(),
            overwatch_handle.clone(),
        )
        .expect("Should not fail to re-create message handler on epoch rotation after public inputs are set.")
    } else {
        current_message_handler
    };

    (new_message_handler, new_public_inputs)
}

/// Processes new secret `PoL` info.
///
/// In case the secret info is received before the public inputs, the message
/// handler is left unchanged. Else, a new message handler is created and
/// returned, that builds on the new epoch's public and private inputs.
fn handle_new_secret_epoch_info<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    new_pol_info: &PolEpochInfo,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_public_inputs: &PoQVerificationInputsMinusSigningKey,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    current_membership: &Membership<NodeId>,
    current_message_handler: MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
) -> MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    RuntimeServiceId: Clone,
{
    // If the public info for the new epoch came in before, create a new message
    // handler with the new info.
    if current_public_inputs.leader.pol_epoch_nonce == new_pol_info.nonce {
        MessageHandler::try_new_with_edge_condition_check(
        settings,
        current_membership.clone(),
        *current_public_inputs,
        new_pol_info.poq_private_inputs.clone(),
        overwatch_handle.clone(),
    )
    .expect("Should not fail to re-create message handler on epoch rotation after private inputs are set.")
    } else {
        current_message_handler
    }
}
