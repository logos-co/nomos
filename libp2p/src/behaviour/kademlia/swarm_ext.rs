use std::collections::HashMap;

use libp2p::{
    Multiaddr, PeerId, StreamProtocol,
    kad::{PeerInfo, QueryId, RecordKey},
};
use rand::RngCore;

use crate::Swarm;

impl<R: Clone + Send + RngCore + 'static> Swarm<R> {
    pub fn get_closest_peers(&mut self, peer_id: PeerId) -> QueryId {
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_get_closest_peers(peer_id)
    }

    pub fn get_kademlia_protocol_names(&self) -> impl Iterator<Item = &StreamProtocol> {
        self.swarm.behaviour().inner.get_kademlia_protocol_names()
    }

    pub fn kademlia_add_address(&mut self, peer_id: PeerId, addr: &Multiaddr) {
        if self
            .global_peer_block_predicate
            .as_ref()
            .is_some_and(|predicate| predicate(peer_id))
        {
            return;
        }
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_add_address(peer_id, addr);
    }

    pub fn kademlia_remove_address(&mut self, peer_id: PeerId, addr: &Multiaddr) {
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_remove_address(peer_id, addr);
    }

    pub fn kademlia_remove_peer(&mut self, peer_id: PeerId) {
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_remove_peer(&peer_id);
    }

    pub fn kademlia_remove_record(&mut self, key: &RecordKey) {
        self.swarm.behaviour_mut().inner.kademlia_remove_record(key);
    }

    pub fn kademlia_remove_provider(&mut self, key: &RecordKey, provider: &PeerId) {
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_remove_provider(key, provider);
    }

    pub fn kademlia_routing_table_dump(&mut self) -> HashMap<u32, Vec<PeerId>> {
        self.kademlia_routing_table_dump_unfiltered()
            .into_iter()
            .map(|(bucket, peers)| {
                (
                    bucket,
                    peers
                        .into_iter()
                        .filter(|peer_id| {
                            !self
                                .global_peer_block_predicate
                                .as_ref()
                                .is_some_and(|predicate| predicate(*peer_id))
                        })
                        .collect(),
                )
            })
            .collect()
    }

    pub fn kademlia_routing_table_dump_unfiltered(&mut self) -> HashMap<u32, Vec<PeerId>> {
        self.swarm
            .behaviour_mut()
            .inner
            .kademlia_routing_table_dump()
    }

    pub fn kademlia_discovered_peers(&mut self) -> Vec<PeerInfo> {
        self.kademlia_discovered_peers_unfiltered()
            .into_iter()
            .filter(|peer_info| {
                !self
                    .global_peer_block_predicate
                    .as_ref()
                    .is_some_and(|predicate| predicate(peer_info.peer_id))
            })
            .collect()
    }

    pub fn kademlia_discovered_peers_unfiltered(&mut self) -> Vec<PeerInfo> {
        self.swarm.behaviour_mut().inner.kademlia_discovered_peers()
    }
}
