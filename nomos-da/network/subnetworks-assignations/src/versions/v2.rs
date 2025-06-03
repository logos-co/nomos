use std::collections::{HashMap, HashSet};

use libp2p::Multiaddr;
use libp2p_identity::PeerId;
use serde::{Deserialize, Serialize};

use crate::MembershipHandler;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct FillWithOriginalReplication {
    assignations: Vec<HashSet<PeerId>>,
    addressbook: HashMap<PeerId, Multiaddr>,
    subnetwork_size: usize,
    dispersal_factor: usize,
    original_replication: usize,
    pivot: u16,
}

impl FillWithOriginalReplication {
    #[must_use]
    pub fn new(
        peers: &[PeerId],
        addressbook: HashMap<PeerId, Multiaddr>,
        subnetwork_size: usize,
        dispersal_factor: usize,
        original_replication: usize,
        pivot: u16,
    ) -> Self {
        Self {
            assignations: Self::fill(
                peers,
                subnetwork_size,
                dispersal_factor,
                original_replication,
                pivot,
            ),
            subnetwork_size,
            dispersal_factor,
            original_replication,
            pivot,
            addressbook,
        }
    }
    fn fill(
        peers: &[PeerId],
        subnetwork_size: usize,
        dispersal_factor: usize,
        original_replication: usize,
        pivot: u16,
    ) -> Vec<HashSet<PeerId>> {
        assert!(!peers.is_empty());
        // sort list to make it deterministic
        let mut peers = peers.to_vec();
        peers.sort_unstable();
        // take n peers and fill a subnetwork until all subnetworks are filled
        let mut cycle = peers.into_iter().cycle();
        (0..subnetwork_size)
            .map(|subnetwork| {
                std::iter::repeat_with(|| cycle.next().unwrap())
                    .take({
                        // choose factor depending on if it is in the original size of the encoding
                        // or not
                        if subnetwork < pivot as usize {
                            original_replication
                        } else {
                            dispersal_factor
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

impl MembershipHandler for FillWithOriginalReplication {
    type NetworkId = u16;
    type Id = PeerId;

    fn new_with(&self, members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>) -> Self {
        Self {
            assignations: Self::fill(
                &members,
                self.subnetwork_size,
                self.dispersal_factor,
                self.original_replication,
                self.pivot,
            ),
            subnetwork_size: self.subnetwork_size,
            dispersal_factor: self.dispersal_factor,
            original_replication: self.original_replication,
            pivot: self.pivot,
            addressbook,
        }
    }

    fn membership(&self, id: &Self::Id) -> HashSet<Self::NetworkId> {
        self.assignations
            .iter()
            .enumerate()
            .filter_map(|(netowrk_id, subnetwork)| {
                subnetwork
                    .contains(id)
                    .then_some(netowrk_id as Self::NetworkId)
            })
            .collect()
    }

    fn is_allowed(&self, id: &Self::Id) -> bool {
        for subnetwork in &self.assignations {
            if subnetwork.contains(id) {
                return true;
            }
        }
        false
    }

    fn members_of(&self, network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.assignations[*network_id as usize].clone()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.assignations.iter().flatten().copied().collect()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        self.subnetwork_size.saturating_sub(1) as u16
    }

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr> {
        self.addressbook.get(peer_id).cloned()
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use libp2p_identity::PeerId;

    use crate::versions::v2::FillWithOriginalReplication;

    #[test]
    fn test_distribution_fill_with_original_replication_from_node_list() {
        let nodes: Vec<_> = std::iter::repeat_with(PeerId::random).take(100).collect();
        let dispersal_factor = 2;
        let subnetwork_size = 1024;
        let original_replication = 10;
        let pivot = 512;
        let distribution = FillWithOriginalReplication::new(
            &nodes,
            HashMap::default(),
            subnetwork_size,
            dispersal_factor,
            original_replication,
            pivot,
        );
        assert_eq!(distribution.assignations.len(), subnetwork_size);
        for subnetwork in &distribution.assignations[..pivot as usize] {
            assert_eq!(subnetwork.len(), original_replication);
        }
        for subnetwork in &distribution.assignations[pivot as usize..] {
            assert_eq!(subnetwork.len(), dispersal_factor);
        }
    }
}
