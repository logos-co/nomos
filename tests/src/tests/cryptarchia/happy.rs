use std::collections::HashSet;

use futures::stream::{self, StreamExt};
use tests::{
    common::cryptarchia::{wait_blocks, CHAIN_LENGTH_MULTIPLIER},
    topology::{Topology, TopologyConfig},
};

async fn happy_test(topology: &Topology) {
    let nodes = topology.validators();
    let config = nodes[0].config();
    let security_param = config.cryptarchia.config.consensus_config.security_param;
    let n_blocks = security_param.get() * CHAIN_LENGTH_MULTIPLIER;

    wait_blocks(n_blocks, nodes, config).await;

    let last_committed_block_height = n_blocks - security_param.get();
    println!("{:?}", nodes[0].consensus_info().await);

    let infos = stream::iter(nodes)
        .then(|n| async move { n.get_headers(None, None).await })
        .map(|blocks| blocks[last_committed_block_height as usize])
        .collect::<HashSet<_>>()
        .await;

    assert_eq!(infos.len(), 1, "consensus not reached");
}

#[tokio::test]
async fn two_nodes_happy() {
    let topology = Topology::spawn(TopologyConfig::two_validators()).await;
    happy_test(&topology).await;
}
