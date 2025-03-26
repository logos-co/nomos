// Temporary code for membership management.

use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};

pub trait ConsensusMembershipHandler {
    type Id;
    fn new() -> Self;

    fn members(&self) -> HashSet<Self::Id>;

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr>;
}

#[derive(Clone)]
pub struct AllNeighbours {
    neighbours: HashSet<PeerId>,
    addresses: HashMap<PeerId, Multiaddr>,
}

impl AllNeighbours {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_neighbour(&mut self, peer_id: PeerId) {
        self.neighbours.insert(peer_id);
    }

    #[cfg(test)]
    pub(crate) fn update_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        self.addresses.insert(peer_id, address);
    }
}

impl Default for AllNeighbours {
    fn default() -> Self {
        Self {
            neighbours: HashSet::new(),
            addresses: HashMap::new(),
        }
    }
}

impl ConsensusMembershipHandler for AllNeighbours {
    type Id = PeerId;

    fn new() -> Self {
        Self::default()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.neighbours.clone()
    }

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr> {
        self.addresses.get(peer_id).cloned()
    }
}
