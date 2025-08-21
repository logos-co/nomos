use core::{ops::RangeInclusive, time::Duration};

use async_trait::async_trait;
use futures::{
    stream::{pending, Pending},
    StreamExt as _,
};
use libp2p::{
    allow_block_list, connection_limits, core::transport::ListenerId, identity::Keypair, Multiaddr,
    PeerId, Swarm,
};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_network::{
    core::{
        with_core::behaviour::{Config as CoreToCoreConfig, IntervalStreamProvider},
        with_edge::behaviour::Config as CoreToEdgeConfig,
        Config, NetworkBehaviour,
    },
    EncapsulatedMessageWithValidatedPublicHeader,
};
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_libp2p::{Protocol, SwarmEvent};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio::{
    sync::{broadcast, mpsc},
    time::interval,
};
use tokio_stream::wrappers::IntervalStream;

use crate::core::{
    backends::libp2p::{behaviour::BlendBehaviour, swarm::BlendSwarmMessage, BlendSwarm},
    settings::BlendConfig,
};

pub struct TestSwarm {
    pub swarm: BlendSwarm<Pending<Membership<PeerId>>, BlakeRng, TestObservationWindowProvider>,
    pub swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    pub incoming_message_receiver:
        broadcast::Receiver<EncapsulatedMessageWithValidatedPublicHeader>,
}

#[derive(Default)]
pub struct SwarmBuilder {
    membership: Option<Membership<PeerId>>,
}

impl SwarmBuilder {
    pub fn with_membership(mut self, membership: Membership<PeerId>) -> Self {
        assert!(self.membership.is_none());
        self.membership = Some(membership);
        self
    }

    pub fn with_empty_membership(mut self) -> Self {
        assert!(self.membership.is_none());
        self.membership = Some(Membership::new(&[], None));
        self
    }

    pub fn build<BehaviourConstructor>(
        self,
        behaviour_constructor: BehaviourConstructor,
    ) -> TestSwarm
    where
        BehaviourConstructor: FnOnce(Keypair) -> BlendBehaviour<TestObservationWindowProvider>,
    {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(100);
        let (incoming_message_sender, incoming_message_receiver) = broadcast::channel(100);

        let swarm = BlendSwarm::new_test(
            behaviour_constructor,
            swarm_message_receiver,
            incoming_message_sender,
            pending(),
            self.membership
                .unwrap_or_else(|| Membership::new(&[], None)),
            BlakeRng::from_entropy(),
            3u64.try_into().unwrap(),
        );

        TestSwarm {
            swarm,
            swarm_message_sender,
            incoming_message_receiver,
        }
    }
}

pub struct BlendBehaviourBuilder {
    peer_id: PeerId,
    membership: Option<Membership<PeerId>>,
    observation_window: Option<(Duration, RangeInclusive<u64>)>,
}

impl BlendBehaviourBuilder {
    pub fn new(identity: &Keypair) -> Self {
        Self {
            peer_id: identity.public().to_peer_id(),
            membership: None,
            observation_window: None,
        }
    }

    pub fn with_membership(mut self, membership: Membership<PeerId>) -> Self {
        assert!(self.membership.is_none());
        self.membership = Some(membership);
        self
    }

    pub fn with_empty_membership(mut self) -> Self {
        assert!(self.membership.is_none());
        self.membership = Some(Membership::new(&[], None));
        self
    }

    pub fn with_observation_window(
        mut self,
        round_duration: Duration,
        expected_message_range: RangeInclusive<u64>,
    ) -> Self {
        self.observation_window = Some((round_duration, expected_message_range));
        self
    }

    pub fn build(self) -> BlendBehaviour<TestObservationWindowProvider> {
        let observation_window_values = self
            .observation_window
            .unwrap_or((Duration::from_secs(1), u64::MIN..=u64::MAX));

        BlendBehaviour {
            blend: NetworkBehaviour::new(
                &Config {
                    with_core: CoreToCoreConfig {
                        peering_degree: 1..=100,
                    },
                    with_edge: CoreToEdgeConfig {
                        connection_timeout: Duration::from_secs(1),
                        max_incoming_connections: 300,
                    },
                },
                TestObservationWindowProvider {
                    expected_message_range: observation_window_values.1,
                    interval: observation_window_values.0,
                },
                self.membership,
                self.peer_id,
            ),
            limits: connection_limits::Behaviour::new(
                connection_limits::ConnectionLimits::default(),
            ),
            blocked_peers: allow_block_list::Behaviour::default(),
        }
    }
}

pub struct TestObservationWindowProvider {
    interval: Duration,
    expected_message_range: RangeInclusive<u64>,
}

#[expect(
    clippy::fallible_impl_from,
    reason = "We need this `From` impl to fulfill the behaviour requirements, but for tests we are actually expect it not to use it."
)]
impl<Settings, NodeId> From<&BlendConfig<Settings, NodeId>> for TestObservationWindowProvider {
    fn from(_: &BlendConfig<Settings, NodeId>) -> Self {
        panic!("This function should never be called in tests since we are hard-coding expected values for the test observation window provider.");
    }
}

impl IntervalStreamProvider for TestObservationWindowProvider {
    type IntervalStream =
        Box<dyn futures::Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let expected_message_range = self.expected_message_range.clone();
        Box::new(
            IntervalStream::new(interval(self.interval))
                .map(move |_| expected_message_range.clone()),
        )
    }
}

#[async_trait]
pub trait SwarmExt: libp2p_swarm_test::SwarmExt {
    async fn listen_and_return_membership_entry(
        &mut self,
        addr: Option<Multiaddr>,
    ) -> (Node<PeerId>, ListenerId);
}

#[async_trait]
impl SwarmExt for Swarm<BlendBehaviour<TestObservationWindowProvider>> {
    async fn listen_and_return_membership_entry(
        &mut self,
        addr: Option<Multiaddr>,
    ) -> (Node<PeerId>, ListenerId) {
        let memory_addr_listener_id = self
            .listen_on(addr.unwrap_or_else(|| Protocol::Memory(0).into()))
            .unwrap();

        // block until we are actually listening
        let address = self
            .wait(|event| match event {
                SwarmEvent::NewListenAddr {
                    address,
                    listener_id,
                } => (listener_id == memory_addr_listener_id).then_some(address),
                other => {
                    panic!("Unexpected event while waiting for `NewListenAddr`: {other:?}")
                }
            })
            .await;
        (
            Node {
                address,
                id: *self.local_peer_id(),
                public_key: Ed25519PrivateKey::generate().public_key(),
            },
            memory_addr_listener_id,
        )
    }
}
