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
use futures::{FutureExt as _, Stream, StreamExt as _};
use nomos_blend_message::crypto::proofs::{
    PoQVerificationInputsMinusSigningKey,
    quota::inputs::prove::{
        private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
        public::LeaderInputs,
    },
};
use nomos_blend_scheduling::{
    message_blend::provers::leader::LeaderProofsGenerator,
    session::{SessionEvent, UninitializedSessionEventStream},
    stream::UninitializedFirstReadyStream,
};
use nomos_core::{codec::SerializeOp as _, crypto::ZkHash};
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
        ChainApi, EpochEvent, EpochHandler, PolEpochInfo, PolInfoProvider as PolInfoProviderTrait,
    },
    membership,
    message::{NetworkMessage, ServiceMessage},
    session::CoreSessionPublicInfo,
    settings::FIRST_STREAM_ITEM_READY_TIMEOUT,
};

const LOG_TARGET: &str = "blend::service::edge";

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
            <MembershipAdapter as membership::Adapter>::Service,
            TimeService<_, _>
        )
        .await?;

        let session_stream = async {
            let membership_stream = MembershipAdapter::new(
                overwatch_handle
                    .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                    .await?,
                settings.crypto.non_ephemeral_signing_key.public_key(),
            )
            .subscribe()
            .expect("Failed to get membership stream from membership service.");

            membership_stream.map(|membership| CoreSessionPublicInfo {
                membership,
                // TODO: Replace with actual session value
                session: 10,
                // TODO: Replace with actual ZK key and merkle path calculation.
                poq_core_inputs: ProofOfCoreQuotaInputs {
                    core_path: ZkHash::ZERO,
                    core_path_selectors: vec![],
                    core_sk: vec![],
                },
            })
        }
        .await;

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

        let messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            NetworkMessage::<BroadcastSettings>::to_bytes(&message)
                .expect("NetworkMessage should be able to be serialized")
                .to_vec()
        });

        run::<Backend, _, ProofsGenerator, _, PolInfoProvider, _>(
            UninitializedSessionEventStream::new(
                session_stream,
                FIRST_STREAM_ITEM_READY_TIMEOUT,
                settings.time.session_transition_period(),
            ),
            UninitializedFirstReadyStream::new(clock_stream, FIRST_STREAM_ITEM_READY_TIMEOUT),
            epoch_handler,
            messages_to_blend,
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
/// - If the secret PoL info is not yielded immediately by the PoL info
///   provider.
async fn run<Backend, NodeId, ProofsGenerator, ChainService, PolInfoProvider, RuntimeServiceId>(
    session_stream: UninitializedSessionEventStream<
        impl Stream<Item = CoreSessionPublicInfo<NodeId>> + Unpin,
    >,
    mut clock_stream: UninitializedFirstReadyStream<impl Stream<Item = SlotTick> + Unpin>,
    mut epoch_handler: EpochHandler<ChainService, RuntimeServiceId>,
    mut messages_to_blend: impl Stream<Item = Vec<u8>> + Send + Unpin,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    notify_ready: impl Fn(),
) -> Result<(), Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    ChainService: ChainApi<RuntimeServiceId>,
    PolInfoProvider: PolInfoProviderTrait<RuntimeServiceId>,
    RuntimeServiceId: Clone,
{
    let (current_session_info, session_stream) = session_stream
        .await_first_ready()
        .await
        .expect("The current session info must be available.");

    let (mut current_epoch_info, mut epoch_stream) = clock_stream
        .first()
        .expect("The node clock must be ready.")
        .map(|tick| {
            epoch_handler
                .tick(tick)
                .expect("The epoch state for the first tick must be available.")
        })
        .await;

    info!(
        target: LOG_TARGET,
        "The current membership is ready: {} nodes.",
        current_session_info.membership.size()
    );

    notify_ready();

    // There might be services that depend on Blend to be ready before starting, so
    // we cannot wait for the stream to be sent before we signal we are
    // ready, hence this should always be called after `notify_ready();`.
    // Also, Blend services start even if such a stream is not immediately
    // available, since they will simply keep blending cover messages.
    let (mut current_secret_pol_info, secret_pol_info_stream) = UninitializedFirstReadyStream::new(
        PolInfoProvider::subscribe(&overwatch_handle)
            .expect("Should not fail to subscribe to secret PoL info stream."),
        FIRST_STREAM_ITEM_READY_TIMEOUT,
    )
    .first()
    .expect("The current secret PoL info should be available for the Blend edge service.");

    let mut message_handler =
        MessageHandler::<Backend, NodeId, ProofsGenerator, RuntimeServiceId>::try_new_with_edge_condition_check(
            settings,
            current_session_info.membership,
            current_epoch_info,
            current_secret_pol_info,
            overwatch_handle.clone(),
        )
        .expect("The initial membership should satisfy the edge node condition");

    loop {
        tokio::select! {
            Some(SessionEvent::NewSession(new_session_info)) = session_stream.next() => {
              message_handler = handle_new_session(new_session_info, settings, current_epoch_info, current_secret_pol_info, overwatch_handle)?;
            }
            Some(message) = messages_to_blend.next() => {
                message_handler.handle_messages_to_blend(message).await;
            }
            Some(tick) = clock_stream.next() => {
                let epoch_event = epoch_handler.tick(tick).await;
                let (EpochEvent::NewEpoch(new_epoch_info) | EpochEvent::NewEpochAndOldEpochTransitionExpired(new_epoch_info)) = epoch_event else {
                    debug!(target: LOG_TARGET, "Skipping epoch event not relevant for the edge service.");
                    continue;
                };
                handle_new_epoch(new_epoch_info, settings, &current_secret_pol_info, &mut current_epoch_info, &current_session_info, &mut message_handler, overwatch_handle.clone());
            }
        }
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] if the membership satisfies all the edge
/// node condition. Otherwise, it returns [`Error`].
fn handle_new_session<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    new_session_info: CoreSessionPublicInfo<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_epoch_public_info: LeaderInputs,
    current_epoch_private_info: ProofOfLeadershipQuotaInputs,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
) -> Result<MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>, Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: LeaderProofsGenerator,
    RuntimeServiceId: Clone,
{
    debug!(target: LOG_TARGET, "Trying to create a new message handler");
    let new_public_inputs = PoQVerificationInputsMinusSigningKey {
        core: new_session_info.poq_core_inputs,
        session: new_session_info.session,
        leader: LeaderInputs {
            message_quota: settings.crypto.num_blend_layers,
            ..current_epoch_public_info
        },
    };
    MessageHandler::<Backend, NodeId, ProofsGenerator, RuntimeServiceId>::try_new_with_edge_condition_check(
        settings,
        new_session_info.membership,
        new_public_inputs,
        current_epoch_private_info,
        overwatch_handle,
    )
}

fn handle_new_epoch<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    new_epoch_info: LeaderInputs,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    current_secret_pol_info: &PolEpochInfo,
    current_epoch_info: &mut LeaderInputs,
    current_session_info: &CoreSessionPublicInfo<NodeId>,
    message_handler: &mut MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
) where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
    ProofsGenerator: LeaderProofsGenerator,
{
    let new_leader_inputs = LeaderInputs {
        message_quota: settings.crypto.num_blend_layers,
        ..new_epoch_info
    };
    *current_epoch_info = PoQVerificationInputsMinusSigningKey {
        leader: new_leader_inputs,
        ..current_session_info
    };
    // Same nonce, all info is provided, we can rotate the epoch.
    if current_secret_pol_info.nonce == new_epoch_info.nonce {
        *message_handler = MessageHandler::<
                    Backend,
                    NodeId,
                    ProofsGenerator,
                    RuntimeServiceId,
                >::try_new_with_edge_condition_check(
                    settings,
                    *current_session_info.membership,
                    *current_epoch_info,
                    *current_secret_pol_info,
                    overwatch_handle,
                )
                .expect("Should not fail to re-create message handler on epoch rotation.");
    }
}

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings>;
