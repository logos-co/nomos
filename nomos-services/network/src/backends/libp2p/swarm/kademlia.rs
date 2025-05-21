use std::collections::HashMap;

use nomos_libp2p::{PeerId, libp2p::kad::PeerInfo};
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
    pub(super) fn handle_discovery_command(&mut self, command: DiscoveryCommand) {
        match command {
            DiscoveryCommand::GetClosestPeers { peer_id, reply } => {
                let _ = self.swarm.get_closest_peers(peer_id, reply);
            }
            DiscoveryCommand::DumpRoutingTable { reply } => {
                let result = self.swarm.kademlia_routing_table_dump();
                tracing::debug!("Routing table dump: {result:?}");
                let _ = reply.send(result);
            }
        }
    }
}
