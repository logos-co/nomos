use futures::stream::{pending, Pending};
use libp2p::PeerId;
use nomos_blend_scheduling::membership::Membership;
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio::sync::mpsc;

use crate::edge::backends::libp2p::BlendSwarm;

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

    pub fn build(self) -> BlendSwarm<Pending<Membership<PeerId>>, BlakeRng> {
        let (_, receiver) = mpsc::channel(100);

        BlendSwarm::new_test(
            self.membership,
            receiver,
            3u64.try_into().unwrap(),
            BlakeRng::from_entropy(),
            pending(),
        )
    }
}
