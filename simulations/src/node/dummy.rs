use std::collections::BTreeSet;

// std
// crates
use crossbeam::channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
// internal
use crate::{
    network::{NetworkInterface, NetworkMessage},
    node::{Node, NodeId},
    overlay::Layout,
};

use super::{OverlayState, SharedState};

#[derive(Debug, Default, Serialize)]
pub struct DummyState {
    pub current_view: usize,
    pub event_one_count: usize,
}

#[derive(Clone, Default, Deserialize)]
pub struct DummySettings {}

#[derive(Clone)]
pub enum DummyMessage {
    EventOne(usize),
    EventTwo(usize),
}

pub struct DummyNode {
    node_id: NodeId,
    state: DummyState,
    settings: DummySettings,
    overlay_state: SharedState<OverlayState>,
    network_interface: DummyNetworkInterface,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DummyRole {
    Root,
    Internal,
    Leaf,
    Unknown,
}

impl DummyNode {
    pub fn new(
        node_id: NodeId,
        overlay_state: SharedState<OverlayState>,
        network_interface: DummyNetworkInterface,
    ) -> Self {
        Self {
            node_id,
            state: Default::default(),
            settings: Default::default(),
            overlay_state,
            network_interface,
        }
    }

    pub fn send_message(&self, address: NodeId, message: DummyMessage) {
        self.network_interface.send_message(address, message);
    }
}

impl Node for DummyNode {
    type Settings = DummySettings;
    type State = DummyState;

    fn id(&self) -> NodeId {
        self.node_id
    }

    fn current_view(&self) -> usize {
        self.state.current_view
    }

    fn state(&self) -> &DummyState {
        &self.state
    }

    fn step(&mut self) {
        let incoming_messages = self.network_interface.receive_messages();
        self.state.current_view += 1;

        for message in incoming_messages {
            match message.payload {
                DummyMessage::EventOne(_) => self.state.event_one_count += 1,
                DummyMessage::EventTwo(_) => todo!(),
            }
        }
    }
}

pub struct DummyNetworkInterface {
    id: NodeId,
    sender: Sender<NetworkMessage<DummyMessage>>,
    receiver: Receiver<NetworkMessage<DummyMessage>>,
}

impl DummyNetworkInterface {
    pub fn new(
        id: NodeId,
        sender: Sender<NetworkMessage<DummyMessage>>,
        receiver: Receiver<NetworkMessage<DummyMessage>>,
    ) -> Self {
        Self {
            id,
            sender,
            receiver,
        }
    }
}

impl NetworkInterface for DummyNetworkInterface {
    type Payload = DummyMessage;

    fn send_message(&self, address: NodeId, message: Self::Payload) {
        let message = NetworkMessage::new(self.id, address, message);
        self.sender.send(message).unwrap();
    }

    fn receive_messages(&self) -> Vec<crate::network::NetworkMessage<Self::Payload>> {
        self.receiver.try_iter().collect()
    }
}

fn get_parent_nodes(node_id: NodeId, layout: &Layout) -> Option<BTreeSet<NodeId>> {
    let committee_id = layout.committee(node_id);
    layout.parent_nodes(committee_id).map(|c| c.nodes)
}

fn get_child_nodes(node_id: NodeId, layout: &Layout) -> Option<BTreeSet<NodeId>> {
    let committee_id = layout.committee(node_id);
    let child_nodes: BTreeSet<NodeId> = layout
        .children_nodes(committee_id)
        .iter()
        .flat_map(|c| c.nodes.clone())
        .collect();
    match child_nodes.len() {
        0 => None,
        _ => Some(child_nodes),
    }
}

fn get_roles(
    parents: &Option<BTreeSet<NodeId>>,
    children: &Option<BTreeSet<NodeId>>,
) -> Vec<DummyRole> {
    match (parents, children) {
        (None, Some(_)) => vec![DummyRole::Root],
        (Some(_), Some(_)) => vec![DummyRole::Internal],
        (Some(_), None) => vec![DummyRole::Leaf],
        (None, None) => vec![DummyRole::Root, DummyRole::Leaf],
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        sync::{Arc, RwLock},
        time::Duration,
    };

    use crossbeam::channel;
    use rand::rngs::mock::StepRng;

    use crate::{
        network::{
            behaviour::NetworkBehaviour,
            regions::{Region, RegionsData},
            Network,
        },
        node::{
            dummy::{get_child_nodes, get_parent_nodes, get_roles, DummyRole},
            NodeId, OverlayState, SharedState, View,
        },
        overlay::{
            tree::{TreeOverlay, TreeSettings},
            Overlay,
        },
    };

    use super::{DummyMessage, DummyNetworkInterface, DummyNode};

    fn init_network(node_ids: &[NodeId]) -> Network<DummyMessage> {
        let regions = HashMap::from([(Region::Europe, node_ids.to_vec())]);
        let behaviour = HashMap::from([(
            (Region::Europe, Region::Europe),
            NetworkBehaviour::new(Duration::from_millis(100), 0.0),
        )]);
        let regions_data = RegionsData::new(regions, behaviour);
        Network::new(regions_data)
    }

    fn init_dummy_nodes(
        node_ids: &[NodeId],
        network: &mut Network<DummyMessage>,
        overlay_state: SharedState<OverlayState>,
    ) -> Vec<DummyNode> {
        node_ids
            .iter()
            .map(|node_id| {
                let (node_message_sender, node_message_receiver) = channel::unbounded();
                let network_message_receiver = network.connect(*node_id, node_message_receiver);
                let network_interface = DummyNetworkInterface::new(
                    *node_id,
                    node_message_sender,
                    network_message_receiver,
                );
                DummyNode::new(*node_id, overlay_state.clone(), network_interface)
            })
            .collect()
    }

    #[test]
    fn get_tree_overlay() {
        let mut rng = StepRng::new(1, 0);
        let overlay = TreeOverlay::new(TreeSettings {
            tree_type: Default::default(),
            depth: 3,
            committee_size: 1,
        });
        let node_ids: Vec<NodeId> = overlay.nodes();
        let mut network = init_network(&node_ids);

        let view = View {
            // leaders: overlay.leaders(&node_ids, 3, &mut rng).collect(),
            leaders: vec![0.into(), 1.into(), 2.into()],
            layout: overlay.layout(&node_ids, &mut rng),
        };
        let overlay_state = Arc::new(RwLock::new(OverlayState {
            views: BTreeMap::from([(1, view)]),
        }));

        let nodes = init_dummy_nodes(&node_ids, &mut network, overlay_state);
    }

    #[test]
    fn get_related_nodes() {
        //       0
        //   1       2
        // 3   4   5   6
        let test_cases = vec![
            (
                0,
                None,
                Some(BTreeSet::from([1.into(), 2.into()])),
                vec![DummyRole::Root],
            ),
            (
                1,
                Some(BTreeSet::from([0.into()])),
                Some(BTreeSet::from([3.into(), 4.into()])),
                vec![DummyRole::Internal],
            ),
            (
                2,
                Some(BTreeSet::from([0.into()])),
                Some(BTreeSet::from([5.into(), 6.into()])),
                vec![DummyRole::Internal],
            ),
            (
                3,
                Some(BTreeSet::from([1.into()])),
                None,
                vec![DummyRole::Leaf],
            ),
            (
                4,
                Some(BTreeSet::from([1.into()])),
                None,
                vec![DummyRole::Leaf],
            ),
            (
                5,
                Some(BTreeSet::from([2.into()])),
                None,
                vec![DummyRole::Leaf],
            ),
            (
                6,
                Some(BTreeSet::from([2.into()])),
                None,
                vec![DummyRole::Leaf],
            ),
        ];
        let mut rng = StepRng::new(1, 0);
        let overlay = TreeOverlay::new(TreeSettings {
            tree_type: Default::default(),
            depth: 3,
            committee_size: 1,
        });
        let node_ids: Vec<NodeId> = overlay.nodes();
        let layout = overlay.layout(&node_ids, &mut rng);

        for (node_id, expected_parents, expected_children, expected_roles) in test_cases {
            let parents = get_parent_nodes(node_id.into(), &layout);
            let children = get_child_nodes(node_id.into(), &layout);
            let role = get_roles(&parents, &children);
            assert_eq!(parents, expected_parents);
            assert_eq!(children, expected_children);
            assert_eq!(role, expected_roles);
        }
    }
}
