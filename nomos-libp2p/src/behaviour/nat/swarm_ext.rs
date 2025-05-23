use libp2p::Multiaddr;

use crate::Swarm;

impl Swarm {
    pub fn add_external_address_candidate(&mut self, addr: Multiaddr) {
        self.swarm
            .behaviour_mut()
            .add_external_address_candidate(addr);
    }
}
