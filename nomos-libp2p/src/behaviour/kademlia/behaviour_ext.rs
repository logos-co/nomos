use std::collections::HashMap;

use libp2p::{kad::QueryId, Multiaddr, PeerId, StreamProtocol};

use crate::behaviour::{Behaviour, BehaviourError};

impl Behaviour {
    pub(crate) fn kademlia_add_address(&mut self, peer_id: PeerId, addr: Multiaddr) {
        let Some(kademlia) = self.kademlia.as_mut() else {
            return;
        };
        kademlia.add_address(&peer_id, addr);
    }

    pub(crate) fn kademlia_routing_table_dump(&mut self) -> HashMap<u32, Vec<PeerId>> {
        let Some(kademlia) = self.kademlia.as_mut() else {
            return HashMap::new();
        };
        kademlia
            .kbuckets()
            .enumerate()
            .map(|(bucket_idx, bucket)| {
                let peers = bucket
                    .iter()
                    .map(|entry| *entry.node.key.preimage())
                    .collect::<Vec<_>>();
                let bucket_idx: u32 = bucket_idx.try_into().expect("Bucket index to be u32 MAX.");
                (bucket_idx, peers)
            })
            .collect()
    }

    pub(crate) fn get_kademlia_protocol_names(&self) -> impl Iterator<Item = &StreamProtocol> {
        self.kademlia
            .as_ref()
            .map_or_else(|| [].iter(), |kademlia| kademlia.protocol_names().iter())
    }

    pub(crate) fn kademlia_get_closest_peers(
        &mut self,
        peer_id: PeerId,
    ) -> Result<QueryId, BehaviourError> {
        let Some(kademlia) = self.kademlia.as_mut() else {
            tracing::error!("kademlia is not enabled");
            return Err(BehaviourError::OperationNotSupported);
        };
        Ok(kademlia.get_closest_peers(peer_id))
    }
}
