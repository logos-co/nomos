use std::{
    fmt::{Debug, Display},
    num::NonZeroU64,
    panic,
    time::Duration,
};

use libp2p::Multiaddr;
use nomos_blend_message::crypto::{Ed25519PrivateKey, Ed25519PublicKey};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
    session::SessionEvent,
    EncapsulatedMessage,
};
use overwatch::overwatch::{commands::OverwatchCommand, OverwatchHandle};
use rand::{rngs::OsRng, RngCore};
use tokio::{sync::mpsc, task::JoinHandle, time::sleep};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    edge::{backends::BlendBackend, handlers::Error, run, settings::BlendConfig},
    settings::TimingSettings,
};

/// [`run`] forwards messages to the core nodes in the updated membership.
#[test_log::test(tokio::test)]
async fn run_with_session_transition() {
    let local_node = 99;
    let mut core_node = 0;
    let minimal_network_size = 1;
    let (_, session_sender, msg_sender, mut node_id_receiver) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&[core_node], local_node)),
    )
    .await;

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
    let core_node = 0;
    let minimal_network_size = 1;
    let (_, session_sender, msg_sender, mut node_id_receiver) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&[core_node], local_node)),
    )
    .await;

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
    let core_nodes = [0];
    let minimal_network_size = 2;
    let (join_handle, _, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&core_nodes, local_node)),
    )
    .await;

    resume_panic_from(join_handle).await;
}

/// [`run`] panics if the local node is not edge in the initial membership.
#[test_log::test(tokio::test)]
#[should_panic(
    expected = "The initial membership should satisfy the edge node condition: LocalIsCoreNode"
)]
async fn run_panics_with_local_is_core_in_initial_membership() {
    let local_node = 99;
    let core_nodes = [local_node];
    let minimal_network_size = 1;
    let (join_handle, _, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&core_nodes, local_node)),
    )
    .await;

    resume_panic_from(join_handle).await;
}

/// [`run`] fails if a new membership is smaller than the minimum network
/// size.
#[test_log::test(tokio::test)]
async fn run_fails_if_new_membership_is_small() {
    let local_node = 99;
    let core_node = 0;
    let minimal_network_size = 1;
    let (join_handle, session_sender, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&[core_node], local_node)),
    )
    .await;

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
    let core_node = 0;
    let minimal_network_size = 1;
    let (join_handle, session_sender, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(membership(&[core_node], local_node)),
    )
    .await;

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

/// [`run`] panics if the session stream does not yield the first event
/// immediately.
#[test_log::test(tokio::test)]
#[should_panic(expected = "Session stream should yield the first event immediately")]
async fn run_panics_if_session_stream_is_not_immediate() {
    let local_node = 99;
    let minimal_network_size = 1;
    // Do not provide the initial membership, so the session stream does not yield
    // immediately.
    let (join_handle, _session_sender, _, _) =
        spawn_run(local_node, minimal_network_size, None).await;

    resume_panic_from(join_handle).await;
}

async fn spawn_run(
    local_node: NodeId,
    minimal_network_size: u64,
    initial_membership: Option<Membership<NodeId>>,
) -> (
    JoinHandle<Result<(), Error>>,
    mpsc::Sender<SessionEvent<Membership<NodeId>>>,
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<NodeId>,
) {
    let (session_sender, session_receiver) = mpsc::channel(1);
    let (msg_sender, msg_receiver) = mpsc::channel(1);
    let (node_id_sender, node_id_receiver) = mpsc::channel(1);

    if let Some(initial_membership) = initial_membership {
        session_sender
            .send(SessionEvent::NewSession(initial_membership))
            .await
            .expect("channel opened");
    }

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
