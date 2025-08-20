use core::{
    iter::repeat_with,
    num::NonZeroU64,
    ops::{Deref, DerefMut, RangeInclusive},
    time::Duration,
};

use futures::{
    stream::{pending, Pending},
    StreamExt as _,
};
use libp2p::{allow_block_list, connection_limits, identity::Keypair, Multiaddr, PeerId};
use nomos_blend_message::{
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, Signature, SIGNATURE_SIZE},
    input::{EncapsulationInput, EncapsulationInputs},
    PayloadType,
};
use nomos_blend_network::{
    core::{
        with_core::behaviour::{Config as CoreToCoreConfig, IntervalStreamProvider},
        with_edge::behaviour::Config as CoreToEdgeConfig,
        Config, NetworkBehaviour,
    },
    EncapsulatedMessageWithValidatedPublicHeader,
};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    EncapsulatedMessage,
};
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
    max_dial_attempts_per_peer: Option<NonZeroU64>,
}

impl SwarmBuilder {
    pub fn with_membership(mut self, membership: Membership<PeerId>) -> Self {
        self.membership = Some(membership);
        self
    }

    pub fn with_max_dial_attempts_per_peer(
        mut self,
        max_dial_attempts_per_peer: NonZeroU64,
    ) -> Self {
        self.max_dial_attempts_per_peer = Some(max_dial_attempts_per_peer);
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
            self.membership.unwrap_or_else(|| {
                Membership::new(
                    &[Node {
                        address: Multiaddr::empty(),
                        id: PeerId::random(),
                        public_key: Ed25519PrivateKey::generate().public_key(),
                    }],
                    None,
                )
            }),
            BlakeRng::from_entropy(),
            self.max_dial_attempts_per_peer
                .unwrap_or_else(|| 3u64.try_into().unwrap()),
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
    peering_degree: Option<RangeInclusive<usize>>,
    membership: Option<Membership<PeerId>>,
    observation_window: Option<(Duration, RangeInclusive<u64>)>,
}

impl BlendBehaviourBuilder {
    pub fn new(identity: &Keypair) -> Self {
        Self {
            peer_id: identity.public().to_peer_id(),
            peering_degree: None,
            membership: None,
            observation_window: None,
        }
    }

    pub fn with_peering_degree(mut self, peering_degree: RangeInclusive<usize>) -> Self {
        self.peering_degree = Some(peering_degree);
        self
    }

    pub fn with_membership(mut self, membership: Membership<PeerId>) -> Self {
        self.membership = Some(membership);
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
                        peering_degree: self.peering_degree.unwrap_or(1..=100),
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

#[derive(Debug)]
pub struct TestEncapsulatedMessage(EncapsulatedMessage);

impl TestEncapsulatedMessage {
    pub fn new(payload: &[u8]) -> Self {
        Self(
            EncapsulatedMessage::new(&generate_valid_inputs(), PayloadType::Data, payload).unwrap(),
        )
    }

    pub fn new_with_invalid_signature(payload: &[u8]) -> Self {
        let mut self_instance = Self::new(payload);
        self_instance.0.public_header_mut().signature = Signature::from([100u8; SIGNATURE_SIZE]);
        self_instance
    }

    pub fn into_inner(self) -> EncapsulatedMessage {
        self.0
    }
}

impl Deref for TestEncapsulatedMessage {
    type Target = EncapsulatedMessage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TestEncapsulatedMessage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

fn generate_valid_inputs() -> EncapsulationInputs<3> {
    EncapsulationInputs::new(
        repeat_with(Ed25519PrivateKey::generate)
            .take(3)
            .map(|recipient_signing_key| {
                let recipient_signing_pubkey = recipient_signing_key.public_key();
                EncapsulationInput::new(
                    Ed25519PrivateKey::generate(),
                    &recipient_signing_pubkey,
                    ProofOfQuota::dummy(),
                    ProofOfSelection::dummy(),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
    .unwrap()
}
