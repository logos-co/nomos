use std::time::Duration;

use futures::stream::{self, StreamExt as _};
use tests::{
    adjust_timeout,
    nodes::validator::{create_validator_config, Validator},
    topology::configs::create_general_configs,
};

#[tokio::test]
async fn test_orphan_handling() {
    let n_validators = 3;

    let general_configs = create_general_configs(n_validators);

    let mut validators = vec![];
    for config in general_configs.iter().take(2) {
        let config = create_validator_config(config.clone());
        validators.push(Validator::spawn(config).await);
    }

    let target_blocks = 10;

    wait_for_validators_mode_and_height(&validators, target_blocks, "Initial validators").await;

    // Start the 3rd node, should catch up via orphan block handling
    println!("Starting 3rd node ...");

    let config = create_validator_config(general_configs[2].clone());

    let behind_nodes = vec![Validator::spawn(config).await];

    println!("Behind node started, waiting for it to sync...");
    tokio::time::sleep(adjust_timeout(Duration::from_secs(2))).await;

    let behind_node_info = behind_nodes[0].consensus_info().await;
    println!("behind node info: {behind_node_info:?}");

    let initial_heights: Vec<_> = stream::iter(&validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect()
        .await;

    let initial_node_min_height = initial_heights.iter().min().unwrap();

    assert!(behind_node_info.height >= *initial_node_min_height - 1);
}

async fn wait_for_validators_mode_and_height(
    validators: &[Validator],
    min_height: u32,
    nodes: &str,
) {
    loop {
        let infos: Vec<_> = stream::iter(validators)
            .then(|n| async move { n.consensus_info().await })
            .collect()
            .await;

        println!(
            "   {nodes}: [{}]",
            infos
                .iter()
                .map(|info| format!("{:?}/", info.height))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let current_min_height = infos.iter().map(|info| info.height).min().unwrap_or(0) as u32;

        if current_min_height >= min_height {
            println!("   All validators reached min height {min_height}",);
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
