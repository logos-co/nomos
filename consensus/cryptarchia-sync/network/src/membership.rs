// Temporary code for membership management.

use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};

pub trait ConsensusMembershipHandler {
    type Id;
    fn new() -> Self;

    fn members(&self) -> HashSet<Self::Id>;

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr>;

    fn add_member(&mut self, peer_id: Self::Id);

    fn remove_member(&mut self, peer_id: &Self::Id);

    fn update_member_address(&mut self, peer_id: Self::Id, address: Multiaddr);
}

#[derive(Clone, Default)]
pub struct AllNeighbours {
    peers: HashSet<PeerId>,
    addresses: HashMap<PeerId, Multiaddr>,
}

impl AllNeighbours {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_neighbour(&mut self, peer_id: PeerId) {
        self.peers.insert(peer_id);
    }

    #[cfg(test)]
    pub(crate) fn update_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        self.addresses.insert(peer_id, address);
    }
}

impl ConsensusMembershipHandler for AllNeighbours {
    type Id = PeerId;

    fn new() -> Self {
        Self::default()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.peers.clone()
    }

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr> {
        self.addresses.get(peer_id).cloned()
    }

    fn add_member(&mut self, peer_id: Self::Id) {
        self.peers.insert(peer_id);
    }

    fn remove_member(&mut self, peer_id: &Self::Id) {
        self.peers.remove(peer_id);
        self.addresses.remove(peer_id);
    }

    fn update_member_address(&mut self, peer_id: Self::Id, address: Multiaddr) {
        self.addresses.insert(peer_id, address);
    }
}
