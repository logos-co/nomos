use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};

/// Keeps track of currently connected peers in the consensus network.
#[derive(Clone, Default)]
pub struct ConnectedPeers {
    peers: HashSet<PeerId>,
    addresses: HashMap<PeerId, Multiaddr>,
}

impl ConnectedPeers {
    /// Initializes new instance with empty set of peers and addresses.
    /// Peers are added to the set when they are connected to the network.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            peers: HashSet::new(),
            addresses: HashMap::new(),
        }
    }
    pub(crate) fn add_peer(&mut self, peer_id: PeerId) {
        self.peers.insert(peer_id);
    }

    pub(crate) fn remove_peer(&mut self, peer_id: &PeerId) {
        self.peers.remove(peer_id);
        self.addresses.remove(peer_id);
    }

    pub(crate) fn update_peer_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        self.addresses.insert(peer_id, address);
    }

    pub(crate) fn all_peers(&self) -> Vec<PeerId> {
        self.peers.iter().copied().collect()
    }
}
