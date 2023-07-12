use consensus_engine::{
    overlay::{FlatOverlay, RoundRobin, TreeOverlay},
    NodeId,
};
use rand::Rng;
use simulations::{
    network::InMemoryNetworkInterface,
    node::carnot::{messages::CarnotMessage, CarnotNode, CarnotSettings},
    settings::SimulationSettings,
};

pub enum OverlayNode {
    Flat(CarnotNode<FlatOverlay<RoundRobin>>),
    Tree(CarnotNode<TreeOverlay<RoundRobin>>),
}

impl OverlayNode {
    pub fn to_overlay_node<R: Rng>(
        node_id: NodeId,
        nodes: Vec<NodeId>,
        leader: NodeId,
        network_interface: InMemoryNetworkInterface<CarnotMessage>,
        genesis: nomos_core::block::Block<[u8; 32]>,
        mut rng: R,
        settings: &SimulationSettings,
    ) -> Self {
        match &settings.overlay_settings {
            simulations::settings::OverlaySettings::Flat => {
                let overlay_settings = consensus_engine::overlay::FlatOverlaySettings {
                    nodes: nodes.to_vec(),
                    leader: RoundRobin::new(),
                    leader_super_majority_threshold: None,
                };
                Self::Flat(CarnotNode::<FlatOverlay<RoundRobin>>::new(
                    node_id,
                    CarnotSettings::new(
                        settings.node_settings.timeout,
                        settings.record_settings.clone(),
                    ),
                    overlay_settings,
                    genesis,
                    network_interface,
                    &mut rng,
                ))
            }
            simulations::settings::OverlaySettings::Tree(tree_settings) => {
                let overlay_settings = consensus_engine::overlay::TreeOverlaySettings {
                    nodes,
                    current_leader: leader,
                    entropy: [0; 32],
                    number_of_committees: tree_settings.number_of_committees,
                    leader: RoundRobin::new(),
                };
                Self::Tree(CarnotNode::<TreeOverlay<RoundRobin>>::new(
                    node_id,
                    CarnotSettings::new(
                        settings.node_settings.timeout,
                        settings.record_settings.clone(),
                    ),
                    overlay_settings,
                    genesis,
                    network_interface,
                    &mut rng,
                ))
            }
        }
    }

    pub fn unwrap_flat(self) -> CarnotNode<FlatOverlay<RoundRobin>> {
        match self {
            OverlayNode::Flat(node) => node,
            _ => panic!("unwrap node with flat overlay failed"),
        }
    }

    pub fn unwrap_tree(self) -> CarnotNode<TreeOverlay<RoundRobin>> {
        match self {
            OverlayNode::Tree(node) => node,
            _ => panic!("unwrap node with flat overlay failed"),
        }
    }
}
