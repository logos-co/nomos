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
use nomos_blend_scheduling::{
    message_blend::{ProofsGenerator as ProofsGeneratorTrait, SessionInfo as PoQSessionInfo},
    session::{SessionEvent, UninitializedSessionEventStream},
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
use tokio::{sync::oneshot, time::timeout};
use tracing::{debug, error, info};

use crate::{
    edge::handlers::{Error, MessageHandler},
    epoch_info::{ChainApi, EpochHandler, PolInfoProvider as PolInfoProviderTrait},
    membership,
    message::{NetworkMessage, ServiceMessage},
    mock_poq_inputs_stream,
    session::SessionInfo,
    settings::FIRST_SESSION_READY_TIMEOUT,
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
    ProofsGenerator: ProofsGeneratorTrait + Send,
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
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        let membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            settings.crypto.non_ephemeral_signing_key.public_key(),
        )
        .subscribe()
        .await?;

        let epoch_handler = async {
            let chain_service = CryptarchiaServiceApi::<ChainService, _>::new(&overwatch_handle)
                .await
                .expect("Failed to establish channel with chain service.");
            EpochHandler::new(chain_service)
        }
        .await;

        // TODO: Change this to also be a `UninitializedStream` which is expected to
        // yield within a certain amount of time.
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

        // TODO: Replace with actual service usage.
        let poq_input_stream = mock_poq_inputs_stream();

        // Stream combining the membership stream with the stream yielding PoQ
        // generation and verification material.
        let aggregated_session_stream = membership_stream.zip(poq_input_stream).map(
            |(membership, (poq_public_inputs, poq_private_inputs))| {
                let local_node_index = membership.local_index();
                let membership_size = membership.size();

                SessionInfo {
                    membership,
                    poq_generation_and_verification_inputs: PoQSessionInfo {
                        local_node_index,
                        membership_size,
                        private_inputs: poq_private_inputs,
                        public_inputs: poq_public_inputs,
                    },
                }
            },
        );

        let uninitialized_session_stream = UninitializedSessionEventStream::new(
            aggregated_session_stream,
            FIRST_SESSION_READY_TIMEOUT,
            settings.time.session_transition_period(),
        );

        let messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            NetworkMessage::<BroadcastSettings>::to_bytes(&message)
                .expect("NetworkMessage should be able to be serialized")
                .to_vec()
        });

        run::<Backend, _, ProofsGenerator, _, PolInfoProvider, _>(
            uninitialized_session_stream,
            clock_stream,
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
async fn run<Backend, NodeId, ProofsGenerator, ChainService, PolInfoProvider, RuntimeServiceId>(
    session_stream: UninitializedSessionEventStream<
        impl Stream<Item = SessionInfo<NodeId>> + Unpin,
    >,
    mut clock_stream: impl Stream<Item = SlotTick> + Unpin,
    mut epoch_handler: EpochHandler<ChainService, RuntimeServiceId>,
    mut messages_to_blend: impl Stream<Item = Vec<u8>> + Send + Unpin,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    notify_ready: impl Fn(),
) -> Result<(), Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    ProofsGenerator: ProofsGeneratorTrait,
    ChainService: ChainApi<RuntimeServiceId>,
    PolInfoProvider: PolInfoProviderTrait<RuntimeServiceId>,
    RuntimeServiceId: Clone,
{
    let (session_info, mut session_stream) = session_stream
        .await_first_ready()
        .await
        .expect("The current session must be ready");

    info!(
        target: LOG_TARGET,
        "The current membership is ready: {} nodes.",
        session_info.membership.size()
    );

    let mut message_handler =
        MessageHandler::<Backend, NodeId, ProofsGenerator, RuntimeServiceId>::try_new_with_edge_condition_check(
            settings,
            session_info,
            overwatch_handle.clone(),
        )
        .expect("The initial membership should satisfy the edge node condition");

    notify_ready();

    // There might be services that depend on Blend to be ready before starting, so
    // we cannot wait for the stream to be sent before we signal we are
    // ready, hence this should always be called after `notify_ready();`.
    // Also, Blend services start even if such a stream is not immediately
    // available, since they will simply keep blending cover messages.
    let _pol_epoch_stream = timeout(
        Duration::from_secs(3),
        PolInfoProvider::subscribe(overwatch_handle)
            .map(|r| r.expect("PoL slot info provider failed to return a usable stream.")),
    )
    .await
    .expect("PoL slot info provider not received within the expected timeout.");

    loop {
        tokio::select! {
            Some(SessionEvent::NewSession(session_info)) = session_stream.next() => {
              message_handler = handle_new_session(session_info, settings, overwatch_handle)?;
            }
            Some(message) = messages_to_blend.next() => {
                message_handler.handle_messages_to_blend(message).await;
            }
            Some(tick) = clock_stream.next() => {
                let new_epoch_info = epoch_handler.tick(tick).await;
                if let Some(new_epoch_info) = new_epoch_info {
                    tracing::trace!(target: LOG_TARGET, "New epoch info received. {new_epoch_info:?}");
                }
            }
        }
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] if the membership satisfies all the edge
/// node condition. Otherwise, it returns [`Error`].
fn handle_new_session<Backend, NodeId, ProofsGenerator, RuntimeServiceId>(
    session_info: SessionInfo<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>, Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    ProofsGenerator: ProofsGeneratorTrait,
    RuntimeServiceId: Clone,
{
    debug!(target: LOG_TARGET, "Trying to create a new message handler");
    MessageHandler::<Backend, NodeId, ProofsGenerator, RuntimeServiceId>::try_new_with_edge_condition_check(
        settings,
        session_info,
        overwatch_handle.clone(),
    )
}

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings>;
