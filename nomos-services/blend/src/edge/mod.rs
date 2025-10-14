pub mod backends;
mod handlers;
pub(crate) mod service_components;
pub mod settings;
#[cfg(test)]
mod tests;

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use backends::BlendBackend;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use futures::{Stream, StreamExt as _};
use nomos_blend_message::crypto::proofs::{
    PoQVerificationInputsMinusSigningKey,
    quota::inputs::prove::{
        private::ProofOfLeadershipQuotaInputs,
        public::{CoreInputs, LeaderInputs},
    },
};
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::provers::leader::LeaderProofsGenerator,
    session::{SessionEvent, UninitializedSessionEventStream},
    stream::UninitializedFirstReadyStream,
};
use nomos_core::codec::SerializeOp as _;
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    OpaqueServiceResourcesHandle,
    overwatch::OverwatchHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        resources::ServiceResourcesHandle,
        state::{NoOperator, NoState},
    },
};
use serde::{Serialize, de::DeserializeOwned};
pub(crate) use service_components::ServiceComponents;
use services_utils::wait_until_services_are_ready;
use settings::BlendConfig;
use tokio::sync::oneshot;
use tracing::{debug, error, info};

use crate::{
    edge::handlers::{Error, MessageHandler},
    epoch_info::{
        ChainApi, EpochEvent, EpochHandler, LeaderInputsMinusQuota, PolEpochInfo,
        PolInfoProvider as PolInfoProviderTrait,
    },
    membership::{self, MembershipInfo},
    message::{NetworkMessage, ServiceMessage},
    settings::FIRST_STREAM_ITEM_READY_TIMEOUT,
};

const LOG_TARGET: &str = "blend::service::edge";

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings>;

pub struct BlendService<
    Backend,
    NodeId,
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(
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
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> ServiceData
    for BlendService<
        Backend,
        NodeId,
        BroadcastSettings,
        MembershipAdapter,
        ProofsGenerator,
        TimeBackend,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type Settings = BlendConfig<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<BroadcastSettings>;
}

#[async_trait::async_trait]
impl<
    Backend,
    NodeId,
    BroadcastSettings,
    MembershipAdapter,
    ProofsGenerator,
    TimeBackend,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for BlendService<
        Backend,
        NodeId,
        BroadcastSettings,
        MembershipAdapter,
        ProofsGenerator,
        TimeBackend,
        ChainService,
        PolInfoProvider,
        RuntimeServiceId,
    >
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + DeserializeOwned + Send,
    MembershipAdapter: membership::Adapter<NodeId = NodeId, Error: Send + Sync + 'static> + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    ProofsGenerator: LeaderProofsGenerator + Send,
    TimeBackend: nomos_time::backends::TimeBackend + Send,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync>,
    PolInfoProvider: PolInfoProviderTrait<RuntimeServiceId, Stream: Send + Unpin + 'static> + Send,
    RuntimeServiceId: AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + AsServiceId<Self>
        + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
        + AsServiceId<ChainService>
        + Display
        + Debug
        + Clone
        + Send
        + Sync
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

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                ServiceResourcesHandle {
                    inbound_relay,
                    overwatch_handle,
                    settings_handle,
                    status_updater,
                    ..
                },
            ..
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            TimeService<_, _>,
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        // Initialize membership stream for session and core-related public PoQ inputs.
        let session_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await
                .expect("Failed to get relay channel with membership service."),
            settings.crypto.non_ephemeral_signing_key.public_key(),
        )
        .subscribe()
        .await
        .expect("Failed to get membership stream from membership service.");

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

        let incoming_message_stream = inbound_relay.map(|ServiceMessage::Blend(message)| {
            NetworkMessage::<BroadcastSettings>::to_bytes(&message)
                .expect("NetworkMessage should be able to be serialized")
                .to_vec()
        });

        let (current_membership_info, session_stream) = UninitializedSessionEventStream::new(
            session_stream,
            FIRST_STREAM_ITEM_READY_TIMEOUT,
            settings.time.session_transition_period(),
        )
        .await_first_ready()
        .await
        .expect("The current session info must be available.");

        let (latest_tick, clock_stream) =
            UninitializedFirstReadyStream::new(clock_stream, FIRST_STREAM_ITEM_READY_TIMEOUT)
                .first()
                .await
                .expect("The clock system must be available.");

        let epoch_handler = async {
            let chain_service = CryptarchiaServiceApi::<ChainService, _>::new(&overwatch_handle)
                .await
                .expect("Failed to establish channel with chain service.");
            EpochHandler::new(
                chain_service,
                settings.time.epoch_transition_period_in_slots,
            )
        }
        .await;

        run::<Backend, _, _, _, ProofsGenerator, _, PolInfoProvider, _>(
            (current_membership_info, session_stream),
            (latest_tick, clock_stream),
            incoming_message_stream,
            epoch_handler,
            &settings,
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
        .map_err(|e| {
            error!(target: LOG_TARGET, "Edge blend service is being terminated with error: {e:?}");
            e.into()
        })
    }
}

/// Run the event loop of the service.
///
/// It listens for new sessions and messages to blend.
/// It recreates the [`MessageHandler`] on each new session to handle messages
/// with the new membership.
/// It returns an [`Error`] if the new membership does not satisfy the edge node
/// condition.
///
/// # Panics
/// - If the initial membership is not yielded immediately from the session
///   stream.
/// - If the initial membership does not satisfy the edge node condition.
/// - If the initial epoch public info is not yielded immediately by the epoch
///   handler.
/// - If the secret `PoL` info is not yielded immediately by the `PoL` info
///   provider.
async fn run<
    Backend,
    NodeId,
    MembershipStream,
    ClockStream,
    ProofsGenerator,
    ChainService,
    PolInfoProvider,
    RuntimeServiceId,
>(
    (current_membership_info, mut session_stream): (MembershipInfo<NodeId>, MembershipStream),
    (latest_tick, mut clock_stream): (ClockStream::Item, ClockStream),
    mut incoming_message_stream: impl Stream<Item = Vec<u8>> + Send + Unpin,
    mut epoch_handler: EpochHandler<ChainService, RuntimeServiceId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    notify_ready: impl Fn(),
) -> Result<(), Error>
where
    MembershipStream: Stream<Item = SessionEvent<MembershipInfo<NodeId>>> + Unpin,
    ClockStream: Stream<Item = SlotTick> + Unpin,
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync + Send,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    ProofsGenerator: LeaderProofsGenerator + Send,
    ChainService: ChainApi<RuntimeServiceId> + Send + Sync,
    PolInfoProvider: PolInfoProviderTrait<RuntimeServiceId, Stream: Unpin>,
    RuntimeServiceId: Clone + Send + Sync,
{
    info!(
        target: LOG_TARGET,
        "The current membership is ready: {} nodes.",
        current_membership_info.membership.size()
    );

    let current_epoch_info = async {
        let EpochEvent::NewEpoch(LeaderInputsMinusQuota {
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        }) = epoch_handler
            .tick(latest_tick)
            .await
            .expect("There should be new epoch state associated with the latest epoch state.")
        else {
            panic!("The first event expected by the epoch handler is a `NewEpoch` event.");
        };
        LeaderInputs {
            message_quota: settings.crypto.num_blend_layers,
            pol_epoch_nonce,
            pol_ledger_aged,
            total_stake,
        }
    }
    .await;

    notify_ready();

    // There might be services that depend on Blend to be ready before starting, so
    // we cannot wait for the stream to be sent before we signal we are
    // ready, hence this should always be called after `notify_ready();`.
    // Also, Blend services start even if such a stream is not immediately
    // available, since they will simply keep blending cover messages.
    let secret_pol_info_stream = UninitializedFirstReadyStream::new(
        PolInfoProvider::subscribe(overwatch_handle)
            .await
            .expect("Should not fail to subscribe to secret PoL info stream."),
        FIRST_STREAM_ITEM_READY_TIMEOUT,
    );
    // TODO: Update the chain leader subscription API to return the latest item
    // immediately, since this service can be started on a new session, which will
    // be mid-epoch. So we can then change this to be an
    // `UninitializedFirstReadyStream` where we expect the latest info to be
    // available immediately.
    let (mut current_private_leader_info, mut secret_pol_info_stream) = secret_pol_info_stream
        .first()
        .await
        .expect("The current epoch secret info must be available.");

    let mut current_public_inputs = PoQVerificationInputsMinusSigningKey {
        core: CoreInputs {
            zk_root: current_membership_info.zk_root,
            // TODO: Replace with actually computed value.
            quota: 1,
        },
        leader: current_epoch_info,
        session: current_membership_info.session_number,
    };

    let mut current_message_handler =
        MessageHandler::<Backend, _, ProofsGenerator, _>::try_new_with_edge_condition_check(
            settings,
            current_membership_info.membership.clone(),
            current_public_inputs,
            current_private_leader_info.poq_private_inputs.clone(),
            overwatch_handle.clone(),
        )
        .expect("The initial membership should satisfy the edge node condition");

    loop {
        tokio::select! {
            Some(SessionEvent::NewSession(new_session_info)) = session_stream.next() => {
              handle_new_session(new_session_info, settings, current_private_leader_info.poq_private_inputs.clone(), overwatch_handle.clone(), &mut current_public_inputs, &mut current_message_handler)?;
            }
            Some(message) = incoming_message_stream.next() => {
                current_message_handler.handle_messages_to_blend(message).await;
            }
            Some(clock_tick) = clock_stream.next() => {
                handle_clock_event(clock_tick, settings, &current_private_leader_info, overwatch_handle, &current_membership_info.membership, &mut epoch_handler, &mut current_public_inputs, &mut current_message_handler).await;
            }
            Some(new_secret_pol_info) = secret_pol_info_stream.next() => {
                handle_new_secret_epoch_info(new_secret_pol_info, settings, &current_public_inputs, overwatch_handle, &current_membership_info.membership, &mut current_private_leader_info, &mut current_message_handler);
            }
        }
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] if the membership satisfies all the edge
/// node condition. Otherwise, it returns [`Error`].
fn handle_new_session<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    MembershipInfo {
        membership: new_membership,
        session_number: new_session_number,
        zk_root: new_zk_root,
    }: MembershipInfo<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_epoch_private_info: ProofOfLeadershipQuotaInputs,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    current_public_inputs: &mut PoQVerificationInputsMinusSigningKey,
    current_message_handler: &mut MessageHandler<
        Backend,
        NodeId,
        ProofsGenerator,
        RuntimeServiceId,
    >,
) -> Result<(), Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    RuntimeServiceId: Clone,
{
    debug!(target: LOG_TARGET, "Trying to create a new message handler");
    // Update current public inputs with new session info.
    *current_public_inputs = PoQVerificationInputsMinusSigningKey {
        session: new_session_number,
        core: CoreInputs {
            // TODO: Replace with actually computed value.
            quota: 1,
            zk_root: new_zk_root,
        },
        leader: current_public_inputs.leader,
    };
    // Replace the old message handler with a new one that is created with the new
    // inputs.
    *current_message_handler = MessageHandler::try_new_with_edge_condition_check(
        settings,
        new_membership,
        *current_public_inputs,
        current_epoch_private_info,
        overwatch_handle,
    )?;

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "TODO: Address this at some point."
)]
async fn handle_clock_event<Backend, NodeId, ProofsGenerator, ChainService, RuntimeServiceId>(
    slot_tick: SlotTick,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    PolEpochInfo {
        nonce: current_secret_inputs_nonce,
        poq_private_inputs: current_secret_inputs,
    }: &PolEpochInfo,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    current_membership: &Membership<NodeId>,
    epoch_handler: &mut EpochHandler<ChainService, RuntimeServiceId>,
    current_public_inputs: &mut PoQVerificationInputsMinusSigningKey,
    current_message_handler: &mut MessageHandler<
        Backend,
        NodeId,
        ProofsGenerator,
        RuntimeServiceId,
    >,
) where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    ProofsGenerator: LeaderProofsGenerator + Send,
    ChainService: ChainApi<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Clone + Send + Sync,
{
    let Some(epoch_event) = epoch_handler.tick(slot_tick).await else {
        return;
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
        EpochEvent::OldEpochTransitionPeriodExpired => {
            return;
        }
    };

    // Update current public inputs with new epoch info.
    current_public_inputs.leader = new_leader_inputs;
    // If the private info for the new epoch came in before, create a new message
    // handler with the new info.
    if current_public_inputs.leader.pol_epoch_nonce == *current_secret_inputs_nonce {
        *current_message_handler = MessageHandler::try_new_with_edge_condition_check(
            settings,
            current_membership.clone(),
            *current_public_inputs,
            current_secret_inputs.clone(),
            overwatch_handle.clone(),
        )
        .expect("Should not fail to re-create message handler on epoch rotation after public inputs are set.");
    }
}

fn handle_new_secret_epoch_info<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    new_pol_info: PolEpochInfo,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_public_inputs: &PoQVerificationInputsMinusSigningKey,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    current_membership: &Membership<NodeId>,
    current_secret_pol_info: &mut PolEpochInfo,
    current_message_handler: &mut MessageHandler<
        Backend,
        NodeId,
        ProofsGenerator,
        RuntimeServiceId,
    >,
) where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    RuntimeServiceId: Clone,
{
    *current_secret_pol_info = new_pol_info;
    // If the public info for the new epoch came in before, create a new message
    // handler with the new info.
    if current_public_inputs.leader.pol_epoch_nonce == current_secret_pol_info.nonce {
        *current_message_handler = MessageHandler::try_new_with_edge_condition_check(
            settings,
            current_membership.clone(),
            *current_public_inputs,
            current_secret_pol_info.poq_private_inputs.clone(),
            overwatch_handle.clone(),
        )
        .expect("Should not fail to re-create message handler on epoch rotation after private inputs are set.");
    }
}
