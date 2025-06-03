use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use arc_swap::ArcSwap;
use libp2p::Multiaddr;
use libp2p_identity::PeerId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{MembershipHandler, UpdateableMembershipHandler};

/// Fill a `N` sized set of "subnetworks" from a list of peer ids members
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillFromNodeList {
    subnetwork_size: usize,
    dispersal_factor: usize,

    #[serde(
        serialize_with = "serialize_arc_arcswap_data",
        deserialize_with = "deserialize_arc_arcswap_data"
    )]
    data: Arc<ArcSwap<NodeListData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeListData {
    assignations: Vec<HashSet<PeerId>>,
    addressbook: HashMap<PeerId, Multiaddr>,
}

// Custom serialization to handle arc swap
fn serialize_arc_arcswap_data<S>(
    data: &Arc<ArcSwap<NodeListData>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Load the current value from the ArcSwap and serialize it
    let current_data = data.load();
    current_data.serialize(serializer)
}

fn deserialize_arc_arcswap_data<'de, D>(
    deserializer: D,
) -> Result<Arc<ArcSwap<NodeListData>>, D::Error>
where
    D: Deserializer<'de>,
{
    let data = NodeListData::deserialize(deserializer)?;
    Ok(Arc::new(ArcSwap::from_pointee(data)))
}

impl FillFromNodeList {
    #[must_use]
    pub fn new(
        peers: &[PeerId],
        addressbook: HashMap<PeerId, Multiaddr>,
        subnetwork_size: usize,
        dispersal_factor: usize,
    ) -> Self {
        let data = NodeListData {
            assignations: Self::fill(peers, subnetwork_size, dispersal_factor),
            addressbook,
        };

        Self {
            data: Arc::new(ArcSwap::from_pointee(data)),
            subnetwork_size,
            dispersal_factor,
        }
    }

    #[must_use]
    pub fn clone_with_different_addressbook(
        &self,
        addressbook: HashMap<PeerId, Multiaddr>,
    ) -> Self {
        let data = self.data.load_full();
        let assignations = data.assignations.clone();

        Self {
            subnetwork_size: self.subnetwork_size,
            dispersal_factor: self.dispersal_factor,
            data: Arc::new(ArcSwap::from_pointee(NodeListData {
                assignations,
                addressbook,
            })),
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
        std::iter::repeat_with(|| {
            std::iter::repeat_with(|| cycle.next().unwrap())
                .take(replication_factor)
                .collect()
        })
        .take(subnetwork_size)
        .collect()
    }
}

impl MembershipHandler for FillFromNodeList {
    type NetworkId = u16;
    type Id = PeerId;

    fn membership(&self, id: &Self::Id) -> HashSet<Self::NetworkId> {
        self.data
            .load()
            .assignations
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
        for subnetwork in &self.data.load_full().assignations {
            if subnetwork.contains(id) {
                return true;
            }
        }
        false
    }

    fn members_of(&self, network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.data.load().assignations[*network_id as usize].clone()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.data
            .load()
            .assignations
            .iter()
            .flatten()
            .copied()
            .collect()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        self.subnetwork_size.saturating_sub(1) as u16
    }

    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr> {
        self.data.load().addressbook.get(peer_id).cloned()
    }
}

impl UpdateableMembershipHandler for FillFromNodeList {
    fn update(&mut self, addressbook: HashMap<PeerId, Multiaddr>) {
        let mut members: Vec<PeerId> = addressbook.keys().copied().collect();
        members.sort_unstable();

        let new_assignations = Self::fill(
            members.as_slice(),
            self.subnetwork_size,
            self.dispersal_factor,
        );
        let new_data = NodeListData {
            assignations: new_assignations,
            addressbook,
        };
        self.data.store(Arc::new(new_data));
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
        assert_eq!(distribution.data.load().assignations.len(), subnetwork_size);
        for subnetwork in &distribution.data.load_full().assignations {
            assert_eq!(subnetwork.len(), dispersal_factor);
        }
    }
}
