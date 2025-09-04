pub mod backends;
mod handlers;
pub(crate) mod service_components;
pub mod settings;

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
    session::{SessionEvent, SessionEventStream},
};
use nomos_core::wire;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{
        resources::ServiceResourcesHandle,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use serde::Serialize;
pub(crate) use service_components::ServiceComponents;
use services_utils::wait_until_services_are_ready;
use settings::BlendConfig;
use tracing::{debug, error, info};

use crate::{
    edge::handlers::{Error, MessageHandler},
    membership,
    message::ServiceMessage,
    settings::constant_membership_stream,
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
    type Settings = BlendConfig<Backend::Settings, NodeId>;
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
    BroadcastSettings: Serialize + Send,
    MembershipAdapter: membership::Adapter + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    <MembershipAdapter as membership::Adapter>::Error: Send + Sync + 'static,
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
        let membership = settings.membership();

        let _membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            settings.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;
        // TODO: Use membership_stream once the membership/SDP services are ready to provide the real membership: https://github.com/logos-co/nomos/issues/1532

        let session_stream = SessionEventStream::new(
            Box::pin(constant_membership_stream(
                membership.clone(),
                settings.time.session_duration(),
            )),
            settings.time.session_transition_period(),
        );

        let messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            wire::serialize(&message)
                .expect("Message from internal services should not fail to serialize")
        });

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        run::<Backend, _, _>(
            session_stream,
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
/// It returns an [`Error`] immediately if the initial membership does not
/// satisfy the edge node condition defined in [`check_edge_condition`].
/// Otherwise, it notifies readiness and starts the event loop.
///
/// In the event loop, it listens for new sessions and messages to blend,
/// It recreates the [`MessageHandler`] on each new session to handle messages
/// with the new membership.
/// It returns an [`Error`] if the new membership does not satisfy the edge node
/// condition.
async fn run<Backend, NodeId, RuntimeServiceId>(
    mut session_stream: impl Stream<Item = SessionEvent<Membership<NodeId>>> + Send + Unpin,
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
    // Read the initial membership, expecting it to be yielded immediately.
    let SessionEvent::NewSession(membership) = session_stream
        .next()
        .await
        .expect("Session stream shouldn't be closed")
    else {
        panic!("NewSession must be yielded first");
    };

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
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings, NodeId>;

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU64, panic};

    use libp2p::Multiaddr;
    use nomos_blend_message::crypto::{Ed25519PrivateKey, Ed25519PublicKey};
    use nomos_blend_scheduling::{
        membership::Node, message_blend::CryptographicProcessorSettings, EncapsulatedMessage,
    };
    use overwatch::overwatch::commands::OverwatchCommand;
    use rand::{rngs::OsRng, RngCore};
    use tokio::{sync::mpsc, task::JoinHandle, time::sleep};
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;
    use crate::settings::TimingSettings;

    /// [`run`] forwards messages to the core nodes in the updated membership.
    #[test_log::test(tokio::test)]
    async fn run_with_session_transition() {
        let local_node = 99;
        let minimal_network_size = 1;
        let (_, session_sender, msg_sender, mut node_id_receiver) =
            spawn_run(local_node, minimal_network_size);

        // Send the initial membership.
        let mut core_node = 0;
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[core_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message should be forwarded to the core node 0.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );

        // Send a new session with another core node 1.
        core_node = 1;
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[core_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message should be forwarded to the core node 1.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );
    }

    /// [`run`] ignores [`SessionEvent::TransitionPeriodExpired`].
    #[test_log::test(tokio::test)]
    async fn run_ignores_transition_period_expired() {
        let local_node = 99;
        let minimal_network_size = 1;
        let (_, session_sender, msg_sender, mut node_id_receiver) =
            spawn_run(local_node, minimal_network_size);

        // Send the initial membership.
        let core_node = 0;
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[core_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // Send a TransitionPeriodExpired that should be ignored.
        session_sender
            .send(SessionEvent::TransitionPeriodExpired)
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message still should be forwarded to the core node 0.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );
    }

    /// [`run`] panics if the initial membership is too small.
    #[test_log::test(tokio::test)]
    #[should_panic(
        expected = "The initial membership should satisfy the edge node condition: NetworkIsTooSmall"
    )]
    async fn run_panics_with_small_initial_membership() {
        let local_node = 99;
        let minimal_network_size = 2;
        let (join_handle, session_sender, _, _) = spawn_run(local_node, minimal_network_size);

        // Send the initial membership, which is too small.
        let core_nodes = [0];
        session_sender
            .send(SessionEvent::NewSession(membership(
                &core_nodes,
                local_node,
            )))
            .await
            .expect("channel opened");

        resume_panic_from(join_handle).await;
    }

    /// [`run`] panics if the local node is not edge in the initial membership.
    #[test_log::test(tokio::test)]
    #[should_panic(
        expected = "The initial membership should satisfy the edge node condition: LocalIsCoreNode"
    )]
    async fn run_panics_with_local_is_core_in_initial_membership() {
        let local_node = 99;
        let minimal_network_size = 1;
        let (join_handle, session_sender, _, _) = spawn_run(local_node, minimal_network_size);

        // Send the initial membership where the local node is core.
        let core_nodes = [local_node];
        session_sender
            .send(SessionEvent::NewSession(membership(
                &core_nodes,
                local_node,
            )))
            .await
            .expect("channel opened");

        resume_panic_from(join_handle).await;
    }

    /// [`run`] fails if a new membership is smaller than the minimum network
    /// size.
    #[test_log::test(tokio::test)]
    async fn run_fails_if_new_membership_is_small() {
        let local_node = 99;
        let minimal_network_size = 1;
        let (join_handle, session_sender, _, _) = spawn_run(local_node, minimal_network_size);

        // Send the initial membership.
        let core_node = 0;
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[core_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // Send a new session with an empty membership (smaller than the min size).
        session_sender
            .send(SessionEvent::NewSession(membership(&[], local_node)))
            .await
            .expect("channel opened");
        assert!(matches!(
            join_handle.await.unwrap(),
            Err(Error::NetworkIsTooSmall(0))
        ));
    }

    /// [`run`] fails if the local node is not edge in a new membership.
    #[test_log::test(tokio::test)]
    async fn run_fails_if_local_is_core_in_new_membership() {
        let local_node = 99;
        let minimal_network_size = 1;
        let (join_handle, session_sender, _, _) = spawn_run(local_node, minimal_network_size);

        // Send the initial membership.
        let core_node = 0;
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[core_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // Send a new session with a membership where the local node is core.
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[local_node],
                local_node,
            )))
            .await
            .expect("channel opened");
        assert!(matches!(
            join_handle.await.unwrap(),
            Err(Error::LocalIsCoreNode)
        ));
    }

    #[expect(clippy::type_complexity, reason = "A test utility function")]
    fn spawn_run(
        local_node: NodeId,
        minimal_network_size: u64,
    ) -> (
        JoinHandle<Result<(), Error>>,
        mpsc::Sender<SessionEvent<Membership<NodeId>>>,
        mpsc::Sender<Vec<u8>>,
        mpsc::Receiver<NodeId>,
    ) {
        let (session_sender, session_receiver) = mpsc::channel(1);
        let (msg_sender, msg_receiver) = mpsc::channel(1);
        let (node_id_sender, node_id_receiver) = mpsc::channel(1);

        let join_handle = tokio::spawn(async move {
            run::<TestBackend, _, _>(
                ReceiverStream::new(session_receiver),
                ReceiverStream::new(msg_receiver),
                &settings(local_node, minimal_network_size, node_id_sender),
                &overwatch_handle(),
                || {},
            )
            .await
        });

        (join_handle, session_sender, msg_sender, node_id_receiver)
    }

    /// Expect the panic from the given async task,
    /// and resume the panic, so the async test can check the panic message.
    async fn resume_panic_from(join_handle: JoinHandle<Result<(), Error>>) {
        panic::resume_unwind(join_handle.await.unwrap_err().into_panic());
    }

    fn membership(ids: &[NodeId], local_id: NodeId) -> Membership<NodeId> {
        Membership::new(
            &ids.iter()
                .map(|id| Node {
                    id: *id,
                    address: Multiaddr::empty(),
                    public_key: key(*id).1,
                })
                .collect::<Vec<_>>(),
            &key(local_id).1,
        )
    }

    fn key(id: NodeId) -> (Ed25519PrivateKey, Ed25519PublicKey) {
        let private_key = Ed25519PrivateKey::from([id; 32]);
        let public_key = private_key.public_key();
        (private_key, public_key)
    }

    fn settings(
        local_id: NodeId,
        minimum_network_size: u64,
        msg_sender: NodeIdSender,
    ) -> BlendConfig<NodeIdSender, NodeId> {
        BlendConfig {
            membership: Vec::new(),
            time: TimingSettings {
                rounds_per_session: NonZeroU64::new(1).unwrap(),
                rounds_per_interval: NonZeroU64::new(1).unwrap(),
                round_duration: Duration::from_secs(1),
                rounds_per_observation_window: NonZeroU64::new(1).unwrap(),
                rounds_per_session_transition_period: NonZeroU64::new(1).unwrap(),
            },
            crypto: CryptographicProcessorSettings {
                signing_private_key: key(local_id).0,
                num_blend_layers: 1,
            },
            backend: msg_sender,
            minimum_network_size: NonZeroU64::new(minimum_network_size).unwrap(),
        }
    }

    type NodeId = u8;
    type NodeIdSender = mpsc::Sender<NodeId>;

    struct TestBackend {
        membership: Membership<NodeId>,
        sender: NodeIdSender,
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> BlendBackend<NodeId, RuntimeServiceId> for TestBackend
    where
        NodeId: Clone,
        RuntimeServiceId: Debug + Sync + Display,
    {
        type Settings = NodeIdSender;

        fn new<Rng>(
            settings: Self::Settings,
            _: OverwatchHandle<RuntimeServiceId>,
            membership: Membership<NodeId>,
            _: Rng,
        ) -> Self
        where
            Rng: RngCore + Send + 'static,
        {
            Self {
                membership,
                sender: settings,
            }
        }

        fn shutdown(self) {}

        async fn send(&self, _: EncapsulatedMessage) {
            let node_id = self
                .membership
                .choose_remote_nodes(&mut OsRng, 1)
                .next()
                .expect("Membership should not be empty")
                .id;
            self.sender.send(node_id).await.unwrap();
        }
    }

    fn overwatch_handle() -> OverwatchHandle<usize> {
        let (sender, _) = mpsc::channel::<OverwatchCommand<usize>>(1);
        OverwatchHandle::new(tokio::runtime::Handle::current(), sender)
    }
}
