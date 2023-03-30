use crate::node::{Node, NodeId};
use crate::output_processors::OutData;
use crate::overlay::Overlay;
use crate::runner::SimulationRunner;
use crate::warding::SimulationState;
use rand::prelude::IteratorRandom;
use std::collections::BTreeSet;
use std::sync::Arc;

/// [Glauber dynamics simulation](https://en.wikipedia.org/wiki/Glauber_dynamics)
pub fn simulate<M, N: Node, O: Overlay>(
    runner: &mut SimulationRunner<M, N, O>,
    update_rate: usize,
    maximum_iterations: usize,
    mut out_data: Option<&mut Vec<OutData>>,
) where
    M: Clone,
    N: Send + Sync,
    N::Settings: Clone,
    O::Settings: Clone,
{
    let simulation_state = SimulationState {
        nodes: Arc::clone(&runner.nodes),
    };
    let nodes_remaining: BTreeSet<NodeId> = (0..runner
        .nodes
        .read()
        .expect("Read access to nodes vector")
        .len())
        .map(From::from)
        .collect();
    let iterations: Vec<_> = (0..maximum_iterations).collect();
    'main: for chunk in iterations.chunks(update_rate) {
        for _ in chunk {
            if nodes_remaining.is_empty() {
                break 'main;
            }

            let node_id = *nodes_remaining.iter().choose(&mut runner.rng).expect(
                "Some id to be selected as it should be impossible for the set to be empty here",
            );

            {
                let mut shared_nodes = runner.nodes.write().expect("Write access to nodes vector");
                let node: &mut N = shared_nodes
                    .get_mut(node_id.inner())
                    .expect("Node should be present");
                node.step();
            }

            // check if any condition makes the simulation stop
            if runner.check_wards(&simulation_state) {
                break 'main;
            }
        }
        runner.dump_state_to_out_data(&simulation_state, &mut out_data);
    }
}
