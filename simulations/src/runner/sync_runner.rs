use serde::Serialize;

use super::{SimulationRunner, SimulationRunnerHandle};
use crate::warding::SimulationState;
use crate::{node::Node, streaming::StreamProducer};
use crossbeam::channel::{bounded, select};
use std::sync::Arc;

/// Simulate with sending the network state to any subscriber
pub fn simulate<M, N: Node, R>(
    runner: SimulationRunner<M, N, R>,
) -> anyhow::Result<SimulationRunnerHandle<R>>
where
    M: Send + Sync + Clone + 'static,
    N: Send + Sync + 'static,
    N::Settings: Clone + Send,
    N::State: Serialize,
    R: for<'a> TryFrom<&'a SimulationState<N>, Error = anyhow::Error> + Send + Sync + 'static,
{
    let state = SimulationState {
        nodes: Arc::clone(&runner.nodes),
    };

    let inner_runner = runner.inner.clone();
    let nodes = runner.nodes;

    let (stop_tx, stop_rx) = bounded(1);
    let p = StreamProducer::<R>::new();
    let p1 = p.clone();
    let handle = std::thread::spawn(move || {
        p.send(R::try_from(&state)?)?;
        loop {
            select! {
                recv(stop_rx) -> _ => {
                    return Ok(());
                }
                default => {
                    let mut inner_runner = inner_runner.write().expect("Write access to inner simulation state");

                    // we must use a code block to make sure once the step call is finished then the write lock will be released, because in Record::try_from(&state),
                    // we need to call the read lock, if we do not release the write lock,
                    // then dead lock will occur
                    {
                        let mut nodes = nodes.write().expect("Write access to nodes vector");
                        inner_runner.step(&mut nodes);
                    }

                    p.send(R::try_from(&state)?)?;
                    // check if any condition makes the simulation stop
                    if inner_runner.check_wards(&state) {
                        return Ok(());
                    }
                }
            }
        }
    });
    Ok(SimulationRunnerHandle {
        producer: p1,
        stop_tx,
        handle,
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        network::{
            behaviour::NetworkBehaviour,
            regions::{Region, RegionsData},
            InMemoryNetworkInterface, Network,
        },
        node::{
            dummy::{DummyMessage, DummyNode},
            Node, NodeId, OverlayState, SharedState, ViewOverlay,
        },
        output_processors::OutData,
        overlay::{tree::TreeOverlay, Overlay, SimulationOverlay},
        runner::SimulationRunner,
        settings::SimulationSettings,
    };
    use crossbeam::channel;
    use rand::rngs::mock::StepRng;
    use std::{
        collections::{BTreeMap, HashMap},
        sync::{Arc, RwLock},
        time::Duration,
    };

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
                let network_interface = InMemoryNetworkInterface::new(
                    *node_id,
                    node_message_sender,
                    network_message_receiver,
                );
                DummyNode::new(*node_id, 0, overlay_state.clone(), network_interface)
            })
            .collect()
    }

    #[test]
    fn runner_one_step() {
        let settings = SimulationSettings {
            node_count: 10,
            ..Default::default()
        };

        let mut rng = StepRng::new(1, 0);
        let node_ids: Vec<NodeId> = (0..settings.node_count).map(Into::into).collect();
        let overlay = TreeOverlay::new(settings.overlay_settings.clone().try_into().unwrap());
        let mut network = init_network(&node_ids);
        let view = ViewOverlay {
            leaders: overlay.leaders(&node_ids, 1, &mut rng).collect(),
            layout: overlay.layout(&node_ids, &mut rng),
        };
        let overlay_state = Arc::new(RwLock::new(OverlayState {
            all_nodes: node_ids.clone(),
            overlay: SimulationOverlay::Tree(overlay),
            overlays: BTreeMap::from([(0, view.clone()), (1, view)]),
        }));
        let nodes = init_dummy_nodes(&node_ids, &mut network, overlay_state);

        let runner: SimulationRunner<DummyMessage, DummyNode, OutData> =
            SimulationRunner::new(network, nodes, settings);
        let mut nodes = runner.nodes.write().unwrap();
        runner.inner.write().unwrap().step(&mut nodes);
        drop(nodes);

        let nodes = runner.nodes.read().unwrap();
        for node in nodes.iter() {
            assert_eq!(node.current_view(), 0);
        }
    }

    #[test]
    fn runner_send_receive() {
        let settings = SimulationSettings {
            node_count: 10,
            ..Default::default()
        };

        let mut rng = StepRng::new(1, 0);
        let node_ids: Vec<NodeId> = (0..settings.node_count).map(Into::into).collect();
        let overlay = TreeOverlay::new(settings.overlay_settings.clone().try_into().unwrap());
        let mut network = init_network(&node_ids);
        let view = ViewOverlay {
            leaders: overlay.leaders(&node_ids, 1, &mut rng).collect(),
            layout: overlay.layout(&node_ids, &mut rng),
        };
        let overlay_state = Arc::new(RwLock::new(OverlayState {
            all_nodes: node_ids.clone(),
            overlay: SimulationOverlay::Tree(overlay),
            overlays: BTreeMap::from([
                (0, view.clone()),
                (1, view.clone()),
                (42, view.clone()),
                (43, view),
            ]),
        }));
        let nodes = init_dummy_nodes(&node_ids, &mut network, overlay_state);

        for node in nodes.iter() {
            // All nodes send one message to NodeId(1).
            // Nodes can send messages to themselves.
            node.send_message(node_ids[1], DummyMessage::Proposal(42.into()));
        }
        network.collect_messages();

        let runner: SimulationRunner<DummyMessage, DummyNode, OutData> =
            SimulationRunner::new(network, nodes, settings);

        let mut nodes = runner.nodes.write().unwrap();
        runner.inner.write().unwrap().step(&mut nodes);
        drop(nodes);

        let nodes = runner.nodes.read().unwrap();
        let state = nodes[1].state();
        assert_eq!(state.message_count, 10);
    }
}
