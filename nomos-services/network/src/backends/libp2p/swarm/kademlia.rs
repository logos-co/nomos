use std::collections::HashMap;

use nomos_libp2p::{Multiaddr, PeerId, Protocol, libp2p::kad::PeerInfo};
use tokio::sync::oneshot;

use crate::backends::libp2p::swarm::SwarmHandler;

#[derive(Debug)]
#[non_exhaustive]
pub enum DiscoveryCommand {
    GetClosestPeers {
        peer_id: PeerId,
        reply: oneshot::Sender<Vec<PeerInfo>>,
    },
    DumpRoutingTable {
        reply: oneshot::Sender<HashMap<u32, Vec<PeerId>>>,
    },
}

impl SwarmHandler {
    pub(super) fn bootstrap_kad_from_peers(&mut self, initial_peers: &Vec<Multiaddr>) {
        for peer_addr in initial_peers {
            if let Some(Protocol::P2p(peer_id_bytes)) = peer_addr.iter().last() {
                if let Ok(peer_id) = PeerId::from_multihash(peer_id_bytes.into()) {
                    self.swarm.kademlia_add_address(peer_id, peer_addr.clone());
                    tracing::debug!("Added peer to Kademlia: {} at {}", peer_id, peer_addr);
                } else {
                    tracing::warn!("Failed to parse peer ID from multiaddr: {}", peer_addr);
                }
            } else {
                tracing::warn!("Multiaddr doesn't contain peer ID: {}", peer_addr);
            }
        }
    }

    pub(super) fn handle_discovery_command(&mut self, command: DiscoveryCommand) {
        match command {
            DiscoveryCommand::GetClosestPeers { peer_id, reply } => {
                match self.swarm.get_closest_peers(peer_id) {
                    Ok(query_id) => {
                        tracing::debug!("Pending query ID: {query_id}");
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Failed to get closest peers for peer ID: {peer_id}: {:?}",
                            err
                        );
                        let _ = reply.send(Vec::new());
                    }
                }
            }
            DiscoveryCommand::DumpRoutingTable { reply } => {
                let result = self.swarm.kademlia_routing_table_dump();
                tracing::debug!("Routing table dump: {result:?}");
                let _ = reply.send(result);
            }
        }
    }
}
