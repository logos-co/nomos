use libp2p::Multiaddr;

use crate::behaviour::Behaviour;

impl Behaviour {
    pub(crate) fn add_external_address_candidate(&mut self, addr: Multiaddr) {
        let Some(nat) = self.nat.as_mut() else {
            return;
        };
        nat.add_external_address_candidate(addr);
    }
}
