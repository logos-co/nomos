use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId, kad::QueryId};

use crate::swarm::behaviour::{BehaviourError, NomosP2pBehaviour};

impl NomosP2pBehaviour {
    pub(crate) fn kademlia_add_address(&mut self, peer_id: PeerId, addr: Multiaddr) {
        if let Some(kademlia) = self.kademlia.as_mut() {
            kademlia.add_address(&peer_id, addr);
        }
    }

    pub(crate) fn kademlia_routing_table_dump(&mut self) -> HashMap<u32, Vec<PeerId>> {
        self.kademlia
            .as_mut()
            .map_or_else(HashMap::new, |kademlia| {
                kademlia
                    .kbuckets()
                    .enumerate()
                    .map(|(bucket_idx, bucket)| {
                        let peers = bucket
                            .iter()
                            .map(|entry| *entry.node.key.preimage())
                            .collect::<Vec<_>>();
                        (bucket_idx as u32, peers)
                    })
                    .collect()
            })
    }

    pub(crate) fn get_kademlia_protocol_names(&self) -> Vec<String> {
        self.kademlia
            .as_ref()
            .map_or_else(std::vec::Vec::new, |kademlia| {
                kademlia
                    .protocol_names()
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            })
    }

    pub(crate) fn kademlia_get_closest_peers(
        &mut self,
        peer_id: PeerId,
    ) -> Result<QueryId, BehaviourError> {
        self.kademlia.as_mut().map_or_else(
            || {
                tracing::error!("kademlia is not enabled");
                Err(BehaviourError::OperationNotSupported)
            },
            |kademlia| Ok(kademlia.get_closest_peers(peer_id)),
        )
    }
}
