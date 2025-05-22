use std::collections::HashMap;

use libp2p::{
    kad::{Event, PeerInfo, ProgressStep, QueryId, QueryResult},
    multiaddr::Protocol,
    Multiaddr, PeerId,
};
use tokio::sync::oneshot;

use crate::swarm::{behaviour::BehaviourError, Swarm};

// Define a struct to hold the data
pub struct PendingQueryData {
    sender: oneshot::Sender<Vec<PeerInfo>>,
    accumulated_results: Vec<PeerInfo>,
}

impl Swarm {
    pub fn get_kademlia_protocol_names(&self) -> Vec<String> {
        self.swarm.behaviour().get_kademlia_protocol_names()
    }

    pub fn bootstrap_kad_from_peers(&mut self, initial_peers: &[Multiaddr]) {
        for peer_addr in initial_peers {
            let Some(Protocol::P2p(peer_id_bytes)) = peer_addr.iter().last() else {
                tracing::warn!("Multiaddr doesn't contain peer ID: {}", peer_addr);
                return;
            };
            let Ok(peer_id) = PeerId::from_multihash(peer_id_bytes.into()) else {
                tracing::warn!("Failed to parse peer ID from multiaddr: {}", peer_addr);
                return;
            };
            self.swarm
                .behaviour_mut()
                .kademlia_add_address(peer_id, peer_addr.clone());
            tracing::debug!("Added peer to Kademlia: {} at {}", peer_id, peer_addr);
        }
    }

    pub fn get_closest_peers(
        &mut self,
        peer_id: libp2p::PeerId,
        sender: oneshot::Sender<Vec<PeerInfo>>,
    ) -> Result<QueryId, BehaviourError> {
        match self
            .swarm
            .behaviour_mut()
            .kademlia_get_closest_peers(peer_id)
        {
            Err(e) => {
                tracing::warn!("Failed to get closest peers for peer ID: {peer_id}: {e:?}",);
                let _ = sender.send(Vec::new());
                Err(e)
            }
            Ok(query_id) => {
                self.pending_queries.insert(
                    query_id,
                    PendingQueryData {
                        sender,
                        accumulated_results: Vec::new(),
                    },
                );
                Ok(query_id)
            }
        }
    }

    pub(crate) fn handle_kademlia_event(&mut self, event: &Event) {
        match event {
            Event::OutboundQueryProgressed {
                id, result, step, ..
            } => {
                self.handle_query_progress(*id, result.clone(), step);
            }
            Event::RoutingUpdated {
                peer,
                addresses,
                old_peer,
                is_new_peer,
                ..
            } => {
                log_routing_update(
                    *peer,
                    &addresses.clone().into_vec(),
                    *old_peer,
                    *is_new_peer,
                );
            }
            event => {
                tracing::debug!("Kademlia event: {:?}", event);
            }
        }
    }

    fn handle_query_progress(&mut self, id: QueryId, result: QueryResult, step: &ProgressStep) {
        match result {
            QueryResult::GetClosestPeers(Ok(result)) => {
                let Some(query_data) = self.pending_queries.get_mut(&id) else {
                    return;
                };
                query_data.accumulated_results.extend(result.peers);

                if step.last {
                    let Some(query_data) = self.pending_queries.remove(&id) else {
                        return;
                    };
                    let _ = query_data.sender.send(query_data.accumulated_results);
                }
            }
            QueryResult::GetClosestPeers(Err(err)) => {
                tracing::warn!("Failed to find closest peers: {:?}", err);
                // For errors, we should probably just send what we have so far
                let Some(query_data) = self.pending_queries.remove(&id) else {
                    return;
                };
                let _ = query_data.sender.send(query_data.accumulated_results);
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
