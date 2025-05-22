use libp2p::identify::Event;

use crate::swarm::Swarm;

impl Swarm {
    pub(crate) fn handle_identify_event(&mut self, event: &Event) {
        let Event::Received { peer_id, info, .. } = event else {
            tracing::debug!("Identify event: {:?}", event);
            return;
        };
        tracing::debug!(
            "Identified peer {} with addresses {:?}",
            peer_id,
            info.listen_addrs
        );
        let kad_protocol_names = self.get_kademlia_protocol_names();
        if info
            .protocols
            .iter()
            .any(|p| kad_protocol_names.contains(&p.to_string()))
        {
            // we need to add the peer to the kademlia routing table
            // in order to enable peer discovery
            for addr in &info.listen_addrs {
                self.swarm
                    .behaviour_mut()
                    .kademlia_add_address(*peer_id, addr.clone());
            }
        }
    }
}
