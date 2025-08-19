use core::num::NonZeroU64;

use futures::stream::{pending, Pending};
use libp2p::{identity::Keypair, Multiaddr, PeerId};
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio::sync::{broadcast, mpsc};

use crate::core::backends::libp2p::{
    behaviour::BlendBehaviour, swarm::BlendSwarmMessage, BlendSwarm,
};

pub struct TestSwarm {
    pub swarm: BlendSwarm<Pending<Membership<PeerId>>, BlakeRng>,
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
        BehaviourConstructor: FnOnce(Keypair) -> BlendBehaviour,
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
