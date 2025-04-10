use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};

#[derive(Clone, Default)]
pub struct ConnectedPeers {
    peers: HashSet<PeerId>,
    addresses: HashMap<PeerId, Multiaddr>,
}

impl ConnectedPeers {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            peers: HashSet::new(),
            addresses: HashMap::new(),
        }
    }
    pub(crate) fn add_member(&mut self, peer_id: PeerId) {
        self.peers.insert(peer_id);
    }

    pub(crate) fn remove_member(&mut self, peer_id: &PeerId) {
        self.peers.remove(peer_id);
        self.addresses.remove(peer_id);
    }

    pub(crate) fn update_member_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        self.addresses.insert(peer_id, address);
    }

    pub(crate) fn members(&self) -> Vec<PeerId> {
        self.peers.iter().copied().collect()
    }
}
