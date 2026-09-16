use std::collections::{HashMap, HashSet};

use lb_libp2p::{
    Multiaddr, PeerId, Protocol,
    libp2p::kad::{self, PeerInfo, ProgressStep, QueryId},
};
use lb_log_targets::network_service;
use rand::RngCore;
use tokio::sync::oneshot;

use crate::backends::libp2p::swarm::SwarmHandler;

const LOG_TARGET: &str = network_service::backends::libp2p::KADEMLIA;

#[derive(Debug)]
#[non_exhaustive]
pub enum DiscoveryCommand {
    GetClosestPeers {
        peer_id: PeerId,
        reply: oneshot::Sender<Vec<PeerInfo>>,
    },
    GetDiscoveredPeers {
        reply: oneshot::Sender<HashSet<PeerId>>,
    },
    DumpRoutingTable {
        reply: oneshot::Sender<HashMap<u32, Vec<PeerId>>>,
    },
}

// Define a struct to hold the data
pub struct PendingQueryData {
    sender: oneshot::Sender<Vec<PeerInfo>>,
    accumulated_results: Vec<PeerInfo>,
}

impl<R: Clone + Send + RngCore + 'static> SwarmHandler<R> {
    #[expect(
        clippy::cognitive_complexity,
        reason = "Retain the existing bootstrap validation and logging in one boundary."
    )]
    pub(super) fn bootstrap_kad_from_peers(&mut self, initial_peers: &Vec<Multiaddr>) {
        for peer_addr in initial_peers {
            if let Some(Protocol::P2p(peer_id_bytes)) = peer_addr.iter().last() {
                if let Ok(peer_id) = PeerId::from_multihash(peer_id_bytes.into()) {
                    if self.is_globally_blocked(peer_id) {
                        continue;
                    }
                    self.swarm.kademlia_add_address(peer_id, peer_addr);
                    tracing::trace!(
                        target: LOG_TARGET,
                        "Added peer to Kademlia: {} at {}",
                        peer_id,
                        peer_addr
                    );
                } else {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Failed to parse peer ID from multiaddr: {}",
                        peer_addr
                    );
                }
            } else {
                tracing::warn!(
                    target: LOG_TARGET,
                    "Multiaddr doesn't contain peer ID: {}",
                    peer_addr
                );
            }
        }
    }

    pub(super) fn handle_discovery_command(&mut self, command: DiscoveryCommand) {
        match command {
            DiscoveryCommand::GetClosestPeers { peer_id, reply } => {
                let query_id = self.swarm.get_closest_peers(peer_id);
                tracing::trace!(target: LOG_TARGET, "Pending query ID: {query_id}");
                self.pending_queries.insert(
                    query_id,
                    PendingQueryData {
                        sender: reply,
                        accumulated_results: Vec::new(),
                    },
                );
            }
            DiscoveryCommand::GetDiscoveredPeers { reply } => {
                let discovered_peers = self
                    .swarm
                    .kademlia_discovered_peers()
                    .into_iter()
                    .map(|peer_info| peer_info.peer_id)
                    .collect::<HashSet<_>>();

                drop(reply.send(discovered_peers));
            }
            DiscoveryCommand::DumpRoutingTable { reply } => {
                let result = self.swarm.kademlia_routing_table_dump();
                tracing::trace!(target: LOG_TARGET, "Routing table dump: {result:?}");
                drop(reply.send(result));
            }
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Keep current Kademlia event handling in one policy boundary."
    )]
    pub(super) fn handle_kademlia_event(&mut self, event: kad::Event) {
        match event {
            kad::Event::RoutablePeer { peer, address }
            | kad::Event::PendingRoutablePeer { peer, address }
                if self.is_globally_blocked(peer) =>
            {
                self.swarm.kademlia_remove_address(peer, &address);
                let _ = self.swarm.disconnect_peer(peer);
            }
            kad::Event::UnroutablePeer { peer } if self.is_globally_blocked(peer) => {
                let _ = self.swarm.disconnect_peer(peer);
            }
            kad::Event::InboundRequest {
                request: kad::InboundRequest::PutRecord { source, .. },
            } if self.is_globally_blocked(source) => {
                // The common GlobalPeerGate drops authenticated handler
                // traffic before Kademlia sees it. Keep this event-level
                // branch as a defensive disconnect for any event already
                // queued by Kademlia; do not remove by record key because
                // that could delete an unrelated local record.
                let _ = self.swarm.disconnect_peer(source);
            }
            kad::Event::InboundRequest {
                request:
                    kad::InboundRequest::AddProvider {
                        record: Some(record),
                    },
            } if self.is_globally_blocked(record.provider) => {
                self.swarm
                    .kademlia_remove_provider(&record.key, &record.provider);
            }
            kad::Event::InboundRequest { request } => {
                // The current libp2p-kad event does not expose the authenticated
                // request peer for FindNode/GetProvider/GetRecord/AddProvider.
                // Those variants are not forwarded to application consumers;
                // the common connection gate and reconciliation handle their
                // peer-attributed admission boundary.
                tracing::trace!(target: LOG_TARGET, "Handle Kademlia inbound request: {request:?}");
            }
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
                if self.is_globally_blocked(peer) {
                    self.swarm.kademlia_remove_peer(peer);
                    let _ = self.swarm.disconnect_peer(peer);
                } else {
                    let addresses = addresses.into_vec();
                    log_routing_update(peer, &addresses, old_peer, is_new_peer);
                    if let Some(old_peer) = old_peer {
                        self.prune_peer_advertised_protocols(old_peer);
                    }
                }
            }
            kad::Event::ModeChanged { new_mode } => {
                tracing::info!(target: LOG_TARGET, "Kademlia mode changed to {new_mode:?}");
            }
            event => {
                tracing::trace!(target: LOG_TARGET, "Kademlia event: {:?}", event);
            }
        }
    }

    pub(super) fn handle_query_progress(
        &mut self,
        id: QueryId,
        result: kad::QueryResult,
        step: &ProgressStep,
    ) {
        match result {
            kad::QueryResult::GetClosestPeers(Ok(result)) => {
                let peers = result
                    .peers
                    .into_iter()
                    .filter(|peer_info| !self.is_globally_blocked(peer_info.peer_id))
                    .collect::<Vec<_>>();
                if let Some(query_data) = self.pending_queries.get_mut(&id) {
                    query_data.accumulated_results.extend(peers);

                    if step.last
                        && let Some(query_data) = self.pending_queries.remove(&id)
                    {
                        drop(query_data.sender.send(query_data.accumulated_results));
                    }
                }
            }
            kad::QueryResult::GetClosestPeers(Err(err)) => {
                tracing::warn!(target: LOG_TARGET, "Failed to find closest peers: {:?}", err);
                // For errors, we should probably just send what we have so far
                if let Some(query_data) = self.pending_queries.remove(&id) {
                    drop(query_data.sender.send(query_data.accumulated_results));
                }
            }
            _ => {
                tracing::trace!(target: LOG_TARGET, "Handle kademlia query result: {:?}", result);
            }
        }
    }
}

fn log_routing_update(
    peer: PeerId,
    address: &[Multiaddr],
    old_peer: Option<PeerId>,
    is_new_peer: bool,
) {
    tracing::trace!(
        target: LOG_TARGET,
        "Routing table updated: peer: {peer}, address: {address:?}, \
         old_peer: {old_peer:?}, is_new_peer: {is_new_peer}"
    );
}
