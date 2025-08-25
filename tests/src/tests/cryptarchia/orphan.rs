use std::time::Duration;

use futures::stream::{self, StreamExt as _};
use tests::{
    adjust_timeout,
    common::sync::wait_for_validators_mode_and_height,
    nodes::validator::{create_validator_config, Validator},
    topology::configs::{
        create_general_configs_with_network,
        network::{Libp2pNetworkLayout, NetworkParams},
    },
};

#[tokio::test]
async fn test_orphan_handling() {
    let n_validators = 3;
    let min_height = 5;

    let network_params = NetworkParams {
        libp2p_network_layout: Libp2pNetworkLayout::Full,
    };
    let general_configs = create_general_configs_with_network(n_validators, &network_params);

    let mut validators = vec![];
    for config in general_configs.iter().take(2) {
        let config = create_validator_config(config.clone());
        validators.push(Validator::spawn(config).await.unwrap());
    }

    println!("Initial validators started: {}", validators.len());

    wait_for_validators_mode_and_height(&validators, cryptarchia_engine::State::Online, min_height)
        .await;

    // Start the 3rd node, should catch up via orphan block handling
    println!("Starting 3rd node ...");

    let config = create_validator_config(general_configs[2].clone());

    let behind_node = vec![Validator::spawn(config).await.unwrap()];

    let mut behind_node_height = 0;

    // Most of the time late node receives an orphan block within a few seconds.
    // But sometimes it takes longer, 20 seconds seems safe.
    tokio::time::timeout(adjust_timeout(Duration::from_secs(20)), async {
        loop {
            let initial_heights: Vec<_> = stream::iter(&validators)
                .then(|n| async move { n.consensus_info().await.height })
                .collect()
                .await;

            // take min because we don't know which node will be the first to send an orphan
            // block
            let initial_node_min_height = initial_heights.iter().min().unwrap();

            let behind_node_info = behind_node[0].consensus_info().await;
            behind_node_height = behind_node_info.height;

            println!("Behind node height: {behind_node_height}");

            if behind_node_height >= *initial_node_min_height - 1 {
                break;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .expect("Timeout waiting for behind node to catch up");
}
