use std::collections::HashMap;

use libp2p::{
    Multiaddr, PeerId,
    kad::{self, PeerInfo, ProgressStep, QueryId},
    multiaddr::Protocol,
};
use tokio::sync::oneshot;

use crate::swarm::{Swarm, behaviour::BehaviourError};

// Define a struct to hold the data
pub struct PendingQueryData {
    sender: oneshot::Sender<Vec<PeerInfo>>,
    accumulated_results: Vec<PeerInfo>,
}

impl Swarm {
    pub fn get_kademlia_protocol_names(&self) -> Vec<String> {
        self.swarm.behaviour().get_kademlia_protocol_names()
    }

    pub fn bootstrap_kad_from_peers(&mut self, initial_peers: &Vec<Multiaddr>) {
        for peer_addr in initial_peers {
            if let Some(Protocol::P2p(peer_id_bytes)) = peer_addr.iter().last() {
                if let Ok(peer_id) = PeerId::from_multihash(peer_id_bytes.into()) {
                    self.swarm
                        .behaviour_mut()
                        .kademlia_add_address(peer_id, peer_addr.clone());
                    tracing::debug!("Added peer to Kademlia: {} at {}", peer_id, peer_addr);
                } else {
                    tracing::warn!("Failed to parse peer ID from multiaddr: {}", peer_addr);
                }
            } else {
                tracing::warn!("Multiaddr doesn't contain peer ID: {}", peer_addr);
            }
        }
    }

    pub fn get_closest_peers(
        &mut self,
        peer_id: libp2p::PeerId,
    ) -> Result<QueryId, BehaviourError> {
        self.swarm
            .behaviour_mut()
            .kademlia_get_closest_peers(peer_id)
    }

    pub fn handle_kademlia_event(&mut self, event: kad::Event) {
        match event {
            kad::Event::OutboundQueryProgressed {
                id, result, step, ..
            } => {
                self.handle_query_progress(id, result, &step);
            }
            kad::Event::RoutingUpdated {
                peer,
                addresses,
                old_peer,
                is_new_peer,
                ..
            } => {
                log_routing_update(peer, &addresses.into_vec(), old_peer, is_new_peer);
            }
            event => {
                tracing::debug!("Kademlia event: {:?}", event);
            }
        }
    }

    pub fn handle_query_progress(
        &mut self,
        id: QueryId,
        result: kad::QueryResult,
        step: &ProgressStep,
    ) {
        match result {
            kad::QueryResult::GetClosestPeers(Ok(result)) => {
                if let Some(query_data) = self.pending_queries.get_mut(&id) {
                    query_data.accumulated_results.extend(result.peers);

                    if step.last {
                        if let Some(query_data) = self.pending_queries.remove(&id) {
                            let _ = query_data.sender.send(query_data.accumulated_results);
                        }
                    }
                }
            }
            kad::QueryResult::GetClosestPeers(Err(err)) => {
                tracing::warn!("Failed to find closest peers: {:?}", err);
                // For errors, we should probably just send what we have so far
                if let Some(query_data) = self.pending_queries.remove(&id) {
                    let _ = query_data.sender.send(query_data.accumulated_results);
                }
            }
            _ => {
                tracing::debug!("Handle kademlia query result: {:?}", result);
            }
        }
    }

    pub fn kademlia_add_address(&mut self, peer_id: PeerId, addr: Multiaddr) {
        self.swarm
            .behaviour_mut()
            .kademlia_add_address(peer_id, addr);
    }

    pub fn kademlia_routing_table_dump(&mut self) -> HashMap<u32, Vec<PeerId>> {
        self.swarm.behaviour_mut().kademlia_routing_table_dump()
    }
}

fn log_routing_update(
    peer: PeerId,
    address: &[Multiaddr],
    old_peer: Option<PeerId>,
    is_new_peer: bool,
) {
    tracing::debug!(
        "Routing table updated: peer: {peer}, address: {address:?}, \
         old_peer: {old_peer:?}, is_new_peer: {is_new_peer}"
    );
}
