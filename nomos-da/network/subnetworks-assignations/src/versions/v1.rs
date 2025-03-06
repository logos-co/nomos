use std::collections::{HashMap, HashSet};

use libp2p::Multiaddr;
use libp2p_identity::PeerId;
use serde::{Deserialize, Serialize};

use crate::MembershipHandler;

/// Fill a `N` sized set of "subnetworks" from a list of peer ids members
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct FillFromNodeList {
    assignations: Vec<HashSet<PeerId>>,
    subnetwork_size: usize,
    dispersal_factor: usize,
    addressbook: HashMap<PeerId, Multiaddr>,
}

impl FillFromNodeList {
    #[must_use]
    pub fn new(
        peers: &[PeerId],
        addressbook: HashMap<PeerId, Multiaddr>,
        subnetwork_size: usize,
        dispersal_factor: usize,
    ) -> Self {
        Self {
            assignations: Self::fill(peers, subnetwork_size, dispersal_factor),
            subnetwork_size,
            dispersal_factor,
            addressbook,
        }
    }

    pub fn clone_with_different_addressbook(
        &self,
        addressbook: HashMap<PeerId, Multiaddr>,
    ) -> Self {
        Self {
            assignations: self.assignations.clone(),
            subnetwork_size: self.subnetwork_size,
            dispersal_factor: self.dispersal_factor,
            addressbook,
        }
    }

    fn fill(
        peers: &[PeerId],
        subnetwork_size: usize,
        replication_factor: usize,
    ) -> Vec<HashSet<PeerId>> {
        assert!(!peers.is_empty());
        // sort list to make it deterministic
        let mut peers = peers.to_vec();
        peers.sort_unstable();
        // take n peers and fill a subnetwork until all subnetworks are filled
        let mut cycle = peers.into_iter().cycle();
        (0..subnetwork_size)
            .map(|_| {
                (0..replication_factor)
                    .map(|_| cycle.next().unwrap())
                    .collect()
            })
            .collect()
    }
}

impl MembershipHandler for FillFromNodeList {
    type NetworkId = u16;
    type Id = PeerId;

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

    use crate::versions::v1::FillFromNodeList;

    #[test]
    fn test_distribution_fill_from_node_list() {
        let nodes: Vec<_> = std::iter::repeat_with(PeerId::random).take(100).collect();
        let dispersal_factor = 2;
        let subnetwork_size = 1024;
        let distribution = FillFromNodeList::new(
            &nodes,
            HashMap::default(),
            subnetwork_size,
            dispersal_factor,
        );
        assert_eq!(distribution.assignations.len(), subnetwork_size);
        for subnetwork in &distribution.assignations {
            assert_eq!(subnetwork.len(), dispersal_factor);
        }
    }
}
