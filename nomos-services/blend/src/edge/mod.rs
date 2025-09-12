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
use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    membership::Membership,
    session::{SessionEvent, UninitializedSessionEventStream},
};
use nomos_core::codec::SerdeOp;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{
        resources::ServiceResourcesHandle,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use serde::{de::DeserializeOwned, Serialize};
pub(crate) use service_components::ServiceComponents;
use services_utils::wait_until_services_are_ready;
use settings::BlendConfig;
use tracing::{debug, error, info};

use crate::{
    edge::handlers::{Error, MessageHandler},
    membership,
    message::{NetworkMessage, ServiceMessage},
    settings::FIRST_SESSION_READY_TIMEOUT,
};

const LOG_TARGET: &str = "blend::service::edge";

pub struct BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<MembershipAdapter>,
}

impl<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
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
impl<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + DeserializeOwned + Send,
    MembershipAdapter: membership::Adapter<NodeId = NodeId, Error: Send + Sync + 'static> + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + AsServiceId<Self>
        + Display
        + Debug
        + Clone
        + Send
        + Sync
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
            settings.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;

        let uninitialized_session_stream = UninitializedSessionEventStream::new(
            membership_stream,
            FIRST_SESSION_READY_TIMEOUT,
            settings.time.session_transition_period(),
        );

        let messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            <NetworkMessage<BroadcastSettings> as SerdeOp>::serialize(&message)
                .expect("NetworkMessage should be able to be serialized")
                .to_vec()
        });

        run::<Backend, _, _>(
            uninitialized_session_stream,
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
async fn run<Backend, NodeId, RuntimeServiceId>(
    session_stream: UninitializedSessionEventStream<impl Stream<Item = Membership<NodeId>> + Unpin>,
    mut messages_to_blend: impl Stream<Item = Vec<u8>> + Send + Unpin,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    notify_ready: impl Fn(),
) -> Result<(), Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    RuntimeServiceId: Clone,
{
    let (membership, mut session_stream) = session_stream
        .await_first_ready()
        .await
        .expect("The current session must be ready");

    info!(
        target: LOG_TARGET,
        "The current membership is ready: {} nodes.",
        membership.size()
    );

    let mut message_handler =
        MessageHandler::<Backend, NodeId, RuntimeServiceId>::try_new_with_edge_condition_check(
            settings,
            membership,
            overwatch_handle.clone(),
        )
        .expect("The initial membership should satisfy the edge node condition");

    notify_ready();

    loop {
        tokio::select! {
            Some(SessionEvent::NewSession(membership)) = session_stream.next() => {
                message_handler = handle_new_session(membership, settings, overwatch_handle)?;
            }
            Some(message) = messages_to_blend.next() => {
                message_handler.handle_messages_to_blend(message).await;
            }
        }
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] if the membership satisfies all the edge
/// node condition. Otherwise, it returns [`Error`].
fn handle_new_session<Backend, NodeId, RuntimeServiceId>(
    membership: Membership<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<MessageHandler<Backend, NodeId, RuntimeServiceId>, Error>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Eq + Hash + Send + 'static,
    RuntimeServiceId: Clone,
{
    debug!(target: LOG_TARGET, "Trying to create a new message handler");
    MessageHandler::<Backend, NodeId, RuntimeServiceId>::try_new_with_edge_condition_check(
        settings,
        membership,
        overwatch_handle.clone(),
    )
}

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings>;
