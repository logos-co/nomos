use std::{
    fmt::{Debug, Display},
    num::NonZeroU64,
    panic,
    time::Duration,
};

use async_trait::async_trait;
use futures::{
    Stream, StreamExt as _,
    stream::{empty, pending},
};
use nomos_blend_scheduling::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::{SessionCryptographicProcessorSettings, SessionInfo as PoQSessionInfo},
    session::UninitializedSessionEventStream,
};
use overwatch::overwatch::{OverwatchHandle, commands::OverwatchCommand};
use rand::{RngCore, rngs::OsRng};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    edge::{backends::BlendBackend, handlers::Error, run, settings::BlendConfig},
    epoch_info::{EpochHandler, PolEpochInfo, PolInfoProvider},
    mock_poq_inputs_stream,
    session::SessionInfo,
    settings::{FIRST_SESSION_READY_TIMEOUT, TimingSettings},
    test_utils::{crypto::MockProofsGenerator, epoch::TestChainService, membership::key},
};

struct EmptyPolStreamProvider;

#[async_trait]
impl<RuntimeServiceId> PolInfoProvider<RuntimeServiceId> for EmptyPolStreamProvider {
    type Stream = Box<dyn Stream<Item = PolEpochInfo> + Send + Unpin>;

    async fn subscribe(
        _overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Option<Self::Stream> {
        Some(Box::new(empty()))
    }
}

pub async fn spawn_run(
    local_node: NodeId,
    minimal_network_size: u64,
    initial_membership: Option<Membership<NodeId>>,
) -> (
    JoinHandle<Result<(), Error>>,
    mpsc::Sender<Membership<NodeId>>,
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<NodeId>,
) {
    let (session_sender, session_receiver) = mpsc::channel(1);
    let (msg_sender, msg_receiver) = mpsc::channel(1);
    let (node_id_sender, node_id_receiver) = mpsc::channel(1);

    if let Some(initial_membership) = initial_membership {
        session_sender
            .send(initial_membership)
            .await
            .expect("channel opened");
    }

    let aggregated_session_stream = ReceiverStream::new(session_receiver)
        .zip(mock_poq_inputs_stream())
        .map(|(membership, (public_inputs, private_inputs))| {
            let local_node_index = membership.local_index();
            let membership_size = membership.size();

            SessionInfo {
                membership,
                poq_generation_and_verification_inputs: PoQSessionInfo {
                    local_node_index,
                    membership_size,
                    private_inputs,
                    public_inputs,
                },
            }
        });

    let join_handle = tokio::spawn(async move {
        Box::pin(run::<
            TestBackend,
            _,
            MockProofsGenerator,
            _,
            EmptyPolStreamProvider,
            _,
        >(
            UninitializedSessionEventStream::new(
                aggregated_session_stream,
                FIRST_SESSION_READY_TIMEOUT,
                // Set 0 for the session transition period since
                // [`SessionEvent::TransitionPeriodExpired`] will be ignored anyway.
                Duration::ZERO,
            ),
            pending(),
            EpochHandler::new(TestChainService),
            ReceiverStream::new(msg_receiver),
            &settings(local_node, minimal_network_size, node_id_sender),
            &overwatch_handle(),
            || {},
        ))
        .await
    });

    (join_handle, session_sender, msg_sender, node_id_receiver)
}

/// Expect the panic from the given async task,
/// and resume the panic, so the async test can check the panic message.
pub async fn resume_panic_from(join_handle: JoinHandle<Result<(), Error>>) {
    panic::resume_unwind(join_handle.await.unwrap_err().into_panic());
}

pub fn settings(
    local_id: NodeId,
    minimum_network_size: u64,
    msg_sender: NodeIdSender,
) -> BlendConfig<NodeIdSender> {
    BlendConfig {
        time: TimingSettings {
            rounds_per_session: NonZeroU64::new(1).unwrap(),
            rounds_per_interval: NonZeroU64::new(1).unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_observation_window: NonZeroU64::new(1).unwrap(),
            rounds_per_session_transition_period: NonZeroU64::new(1).unwrap(),
        },
        crypto: SessionCryptographicProcessorSettings {
            non_ephemeral_signing_key: key(local_id).0,
            num_blend_layers: 1,
        },
        backend: msg_sender,
        minimum_network_size: NonZeroU64::new(minimum_network_size).unwrap(),
    }
}

pub type NodeIdSender = mpsc::Sender<NodeId>;

pub struct TestBackend {
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

pub fn overwatch_handle() -> OverwatchHandle<usize> {
    let (sender, _) = mpsc::channel::<OverwatchCommand<usize>>(1);
    OverwatchHandle::new(tokio::runtime::Handle::current(), sender)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct NodeId(pub u8);

impl From<NodeId> for [u8; 32] {
    fn from(id: NodeId) -> Self {
        [id.0; 32]
    }
}
