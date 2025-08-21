use futures::stream::{pending, Pending};
use libp2p::PeerId;
use nomos_blend_scheduling::membership::Membership;
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio::sync::mpsc;

use crate::edge::backends::libp2p::{swarm::Command, BlendSwarm};

pub struct TestSwarm {
    pub swarm: BlendSwarm<Pending<Membership<PeerId>>, BlakeRng>,
    pub command_sender: mpsc::Sender<Command>,
}

#[derive(Default)]
pub struct SwarmBuilder {
    membership: Option<Membership<PeerId>>,
}

impl SwarmBuilder {
    pub fn with_membership(mut self, membership: Membership<PeerId>) -> Self {
        self.membership = Some(membership);
        self
    }

    pub fn build(self) -> TestSwarm {
        let (command_sender, command_receiver) = mpsc::channel(100);

        let swarm = BlendSwarm::new_test(
            self.membership,
            command_receiver,
            3u64.try_into().unwrap(),
            BlakeRng::from_entropy(),
            pending(),
        );

        TestSwarm {
            swarm,
            command_sender,
        }
    }
}
