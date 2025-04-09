use tests::{
    common::cryptarchia::wait_blocks,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn sync_test_no_forks() {
    let topology_config = TopologyConfig::validators(3);
    let (validators_configs, _) = Topology::create_configs(&topology_config);
    let bootstrap_topology =
        Topology::spawn_with_config(validators_configs[0..2].to_vec(), vec![]).await;

    let n_blocks = 5;
    let node = &bootstrap_topology.validators()[0];

    wait_blocks(n_blocks, bootstrap_topology.validators(), node.config()).await;

    let topology = Topology::spawn_with_config(vec![validators_configs[2].clone()], vec![]).await;
    wait_blocks(n_blocks, topology.validators(), node.config()).await;

    // Assert node 2 height is the same as node 0
    let node_2 = &topology.validators()[0];
    let node_0 = &bootstrap_topology.validators()[0];
    let node_0_height = node_0.consensus_info().await.height;
    let node_2_height = node_2.consensus_info().await.height;
    assert_eq!(node_0_height, node_2_height);
}
