use std::{num::NonZeroU64, pin::Pin, time::Duration};

use futures::{stream::pending, Stream, StreamExt as _};
use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::{
    membership::Membership, message_blend::CryptographicProcessorSettings, session::SessionEvent,
    EncapsulatedMessage,
};
use nomos_network::{backends::NetworkBackend, NetworkService};
use nomos_utils::{blake_rng::BlakeRng, math::NonNegativeF64};
use overwatch::{
    overwatch::{commands::OverwatchCommand, OverwatchHandle},
    services::{relay::OutboundRelay, ServiceData},
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

use crate::{
    core::{
        backends::BlendBackend,
        handlers::Error,
        network::NetworkAdapter,
        run,
        settings::{
            BlendConfig, CoverTrafficSettingsExt, MessageDelayerSettingsExt, SchedulerSettingsExt,
        },
    },
    message::ServiceMessage,
    settings::TimingSettings,
    test_utils::{
        membership::{key, membership as new_membership, NodeId},
        panic::resume_panic_from,
    },
};

/// [`run`] panics if the initial membership is too small.
#[tokio::test]
#[should_panic(
    expected = "The initial membership should satisfy the core node condition: NetworkIsTooSmall"
)]
async fn run_panics_if_initial_membership_is_too_small() {
    let local_node = 99;
    let core_nodes = [local_node];
    let minimal_network_size = 2;
    let (join_handle, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(new_membership(&core_nodes, local_node)),
    )
    .await;

    resume_panic_from(join_handle).await;
}

/// [`run`] panics if the local node is not core in the initial membership.
#[tokio::test]
#[should_panic(
    expected = "The initial membership should satisfy the core node condition: LocalIsNotCoreNode"
)]
async fn run_panics_with_local_is_not_core_in_initial_membership() {
    let local_node = 99;
    let core_nodes = [0];
    let minimal_network_size = 1;
    let (join_handle, _, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(new_membership(&core_nodes, local_node)),
    )
    .await;

    resume_panic_from(join_handle).await;
}

/// [`run`] fails if a new membership is smaller than the minimum network
/// size.
#[tokio::test]
async fn run_fails_if_new_membership_is_small() {
    let local_node = 99;
    let core_node = 0;
    let core_nodes = [core_node, local_node];
    let minimal_network_size = 2;
    let (join_handle, session_sender, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(new_membership(&core_nodes, local_node)),
    )
    .await;

    // Send a new session with a membership smaller than the min size
    session_sender
        .send(SessionEvent::NewSession(new_membership(
            &[local_node],
            local_node,
        )))
        .await
        .expect("channel opened");
    assert!(matches!(
        join_handle.await.unwrap(),
        Err(Error::NetworkIsTooSmall(1))
    ));
}

/// [`run`] fails if the local node is not core in a new membership.
#[tokio::test]
async fn run_fails_if_local_is_not_core_in_new_membership() {
    let local_node = 99;
    let core_nodes = [local_node];
    let minimal_network_size = 1;
    let (join_handle, session_sender, _) = spawn_run(
        local_node,
        minimal_network_size,
        Some(new_membership(&core_nodes, local_node)),
    )
    .await;

    // Send a new session with a membership where the local node is not core.
    let core_nodes = [0];
    session_sender
        .send(SessionEvent::NewSession(new_membership(
            &core_nodes,
            local_node,
        )))
        .await
        .expect("channel opened");
    assert!(matches!(
        join_handle.await.unwrap(),
        Err(Error::LocalIsNotCoreNode)
    ));
}

async fn spawn_run(
    local_node: NodeId,
    minimal_network_size: u64,
    initial_membership: Option<Membership<NodeId>>,
) -> (
    JoinHandle<Result<(), Error>>,
    mpsc::Sender<SessionEvent<Membership<NodeId>>>,
    mpsc::Sender<ServiceMessage<()>>,
) {
    let (session_sender, session_receiver) = mpsc::channel(1);
    let (local_msg_sender, local_msg_receiver) = mpsc::channel(1);

    if let Some(initial_membership) = initial_membership {
        session_sender
            .send(SessionEvent::NewSession(initial_membership))
            .await
            .expect("channel opened");
    }

    let join_handle = tokio::spawn(async move {
        run::<TestBlendBackend, _, _, _>(
            ReceiverStream::new(session_receiver),
            ReceiverStream::new(local_msg_receiver),
            TestNetworkAdapter,
            &settings(local_node, minimal_network_size),
            &overwatch_handle(),
            || {},
        )
        .await
    });

    (join_handle, session_sender, local_msg_sender)
}

fn settings(local_id: NodeId, minimum_network_size: u64) -> BlendConfig<(), NodeId> {
    BlendConfig {
        backend: (),
        crypto: CryptographicProcessorSettings {
            signing_private_key: key(local_id).0,
            num_blend_layers: 1,
        },
        scheduler: SchedulerSettingsExt {
            cover: CoverTrafficSettingsExt {
                message_frequency_per_round: NonNegativeF64::try_from(1.0).unwrap(),
                redundancy_parameter: 0,
                intervals_for_safety_buffer: 0,
            },
            delayer: MessageDelayerSettingsExt {
                maximum_release_delay_in_rounds: NonZeroU64::new(1).unwrap(),
            },
        },
        time: TimingSettings {
            rounds_per_session: NonZeroU64::new(1).unwrap(),
            rounds_per_interval: NonZeroU64::new(1).unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_observation_window: NonZeroU64::new(1).unwrap(),
            rounds_per_session_transition_period: NonZeroU64::new(1).unwrap(),
        },
        membership: Vec::new(),
        minimum_network_size: NonZeroU64::new(minimum_network_size).unwrap(),
    }
}

struct TestBlendBackend;

#[async_trait::async_trait]
impl<RuntimeServiceId> BlendBackend<NodeId, BlakeRng, RuntimeServiceId> for TestBlendBackend {
    type Settings = ();

    fn new(
        _: BlendConfig<Self::Settings, NodeId>,
        _: OverwatchHandle<RuntimeServiceId>,
        _: Membership<NodeId>,
        _: Pin<Box<dyn Stream<Item = SessionEvent<Membership<NodeId>>> + Send>>,
        _: BlakeRng,
    ) -> Self {
        Self
    }

    fn shutdown(self) {}

    async fn publish(&self, _: EncapsulatedMessage) {}

    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = EncapsulatedMessageWithValidatedPublicHeader> + Send>> {
        pending().boxed()
    }
}

struct TestNetworkAdapter;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for TestNetworkAdapter {
    type Backend = TestNetworkBackend;
    type BroadcastSettings = ();

    fn new(
        _: OutboundRelay<<NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message>,
    ) -> Self {
        Self
    }

    async fn broadcast(&self, _: Vec<u8>, _: Self::BroadcastSettings) {
        unimplemented!()
    }
}

struct TestNetworkBackend;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for TestNetworkBackend {
    type Settings = ();
    type Message = ();
    type PubSubEvent = ();
    type ChainSyncEvent = ();

    fn new((): Self::Settings, _: OverwatchHandle<RuntimeServiceId>) -> Self {
        Self
    }

    async fn process(&self, _: Self::Message) {
        unimplemented!()
    }

    async fn subscribe_to_pubsub(&mut self) -> BroadcastStream<Self::PubSubEvent> {
        unimplemented!()
    }

    async fn subscribe_to_chainsync(&mut self) -> BroadcastStream<Self::ChainSyncEvent> {
        unimplemented!()
    }
}

fn overwatch_handle() -> OverwatchHandle<usize> {
    let (sender, _) = mpsc::channel::<OverwatchCommand<usize>>(1);
    OverwatchHandle::new(tokio::runtime::Handle::current(), sender)
}
