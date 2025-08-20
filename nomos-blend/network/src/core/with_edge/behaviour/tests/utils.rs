use core::time::Duration;
use std::collections::{HashSet, VecDeque};

use async_trait::async_trait;
use libp2p::{Multiaddr, PeerId, Stream, Swarm};
use libp2p_stream::Behaviour as StreamBehaviour;
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::{Membership, Node};

use crate::{core::with_edge::behaviour::Behaviour, PROTOCOL_NAME};

#[derive(Default)]
pub struct BehaviourBuilder {
    edge_peer_ids: Vec<PeerId>,
    max_incoming_connections: Option<usize>,
    timeout: Option<Duration>,
}

impl BehaviourBuilder {
    pub fn with_edge_peer_membership(mut self, edge_peer_id: PeerId) -> Self {
        self.edge_peer_ids.push(edge_peer_id);
        self
    }

    pub fn with_max_incoming_connections(mut self, max_incoming_connections: usize) -> Self {
        self.max_incoming_connections = Some(max_incoming_connections);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn build(self) -> Behaviour {
        let current_membership = if self.edge_peer_ids.is_empty() {
            None
        } else {
            Some(Membership::new(
                self.edge_peer_ids
                    .into_iter()
                    .map(|edge_peer_id| Node {
                        address: Multiaddr::empty(),
                        id: edge_peer_id,
                        public_key: Ed25519PrivateKey::generate().public_key(),
                    })
                    .collect::<Vec<_>>()
                    .as_ref(),
                None,
            ))
        };
        Behaviour {
            events: VecDeque::new(),
            waker: None,
            current_membership,
            connection_timeout: self.timeout.unwrap_or(Duration::from_secs(1)),
            upgraded_edge_peers: HashSet::new(),
            max_incoming_connections: self.max_incoming_connections.unwrap_or(100),
        }
    }
}

#[async_trait]
pub trait StreamBehaviourExt: libp2p_swarm_test::SwarmExt {
    async fn connect_and_upgrade_to_blend(&mut self, other: &mut Swarm<Behaviour>) -> Stream;
}

#[async_trait]
impl StreamBehaviourExt for Swarm<StreamBehaviour> {
    async fn connect_and_upgrade_to_blend(&mut self, other: &mut Swarm<Behaviour>) -> Stream {
        // We connect and return the stream preventing it from being dropped so the
        // blend node does not close the connection with an EOF error.
        self.connect(other).await;
        self.behaviour_mut()
            .new_control()
            .open_stream(*other.local_peer_id(), PROTOCOL_NAME)
            .await
            .unwrap()
    }
}
