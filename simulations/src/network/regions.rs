// std
use std::collections::HashMap;
// crates
use serde::{Deserialize, Serialize};
// internal
use crate::{network::behaviour::NetworkBehaviour, node::NodeId};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Region {
    NorthAmerica,
    Europe,
    Asia,
    Africa,
    SouthAmerica,
    Australia,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegionsData {
    pub regions: HashMap<Region, Vec<NodeId>>,
    #[serde(skip)]
    pub node_region: HashMap<NodeId, Region>,
    pub region_network_behaviour: HashMap<(Region, Region), NetworkBehaviour>,
}

impl RegionsData {
    pub fn new(
        regions: HashMap<Region, Vec<NodeId>>,
        region_network_behaviour: HashMap<(Region, Region), NetworkBehaviour>,
    ) -> Self {
        let node_region = regions
            .iter()
            .flat_map(|(region, nodes)| nodes.iter().copied().map(|node| (node, *region)))
            .collect();
        Self {
            regions,
            node_region,
            region_network_behaviour,
        }
    }

    pub fn node_region(&self, node_id: NodeId) -> Region {
        self.node_region[&node_id]
    }

    pub fn network_behaviour(&self, node_a: NodeId, node_b: NodeId) -> &NetworkBehaviour {
        let region_a = self.node_region[&node_a];
        let region_b = self.node_region[&node_b];
        self.region_network_behaviour
            .get(&(region_a, region_b))
            .or(self.region_network_behaviour.get(&(region_b, region_a)))
            .expect("Network behaviour not found for the given regions")
    }

    pub fn region_nodes(&self, region: Region) -> &[NodeId] {
        &self.regions[&region]
    }
}
