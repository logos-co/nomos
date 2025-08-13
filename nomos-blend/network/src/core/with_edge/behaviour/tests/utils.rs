use core::time::Duration;
use std::collections::{HashSet, VecDeque};

use libp2p::{Multiaddr, PeerId};
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::{Membership, Node};

use crate::core::with_edge::behaviour::Behaviour;

impl Behaviour {
    #[must_use]
    pub fn with_no_membership() -> Self {
        Self {
            connection_timeout: Duration::from_secs(1),
            current_membership: None,
            events: VecDeque::new(),
            max_incoming_connections: 100,
            upgraded_edge_peers: HashSet::new(),
            waker: None,
        }
    }

    #[must_use]
    pub fn with_edge_peer(edge_peer_id: PeerId) -> Self {
        let mut self_instance = Self::with_no_membership();
        self_instance.current_membership = Some(Membership::new(
            &[Node {
                address: Multiaddr::empty(),
                id: edge_peer_id,
                public_key: Ed25519PrivateKey::generate().public_key(),
            }],
            None,
        ));

        self_instance
    }
}
