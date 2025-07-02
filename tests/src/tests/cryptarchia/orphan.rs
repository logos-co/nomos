use std::{collections::HashSet, time::Duration};

use futures::stream::{self, StreamExt as _};
use tests::{
    adjust_timeout,
    nodes::validator::{create_validator_config, Validator},
    topology::configs::create_general_configs,
};
use tokio::time::{sleep, timeout};

const CHAIN_LENGTH_MULTIPLIER: u32 = 2;

#[tokio::test]
async fn test_orphan_stream() {
    let n_validators = 3;

    let general_configs = create_general_configs(n_validators);
    let consensus_params = general_configs[0]
        .consensus_config
        .ledger_config
        .consensus_config;

    let mut validators = vec![];
    for config in general_configs.iter().take(2) {
        let config = create_validator_config(config.clone());
        validators.push(Validator::spawn(config).await);
    }

    let security_param = consensus_params.security_param.get();
    let target_blocks = security_param * CHAIN_LENGTH_MULTIPLIER;
    let min_blocks = 5;

    wait_for_validators_mode_and_height(&validators, target_blocks, "Initial validators").await;

    // Start the 3rd node with initial peers (should catch up via IBD)
    println!("Starting 3rd node ...");

    let mut config = create_validator_config(general_configs[2].clone());

    let initial_heights: Vec<_> = stream::iter(&validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect()
        .await;

    let initial_node_min_height = initial_heights.iter().min().unwrap();

    let behind_nodes = vec![Validator::spawn(config).await];

    println!("Behind node started, waiting for it to sync...");
    tokio::time::sleep(adjust_timeout(Duration::from_secs(5))).await;

    let behind_node_info = behind_nodes[0].consensus_info().await;
    println!("behind node info: {behind_node_info:?}");

    let found_common = timeout(Duration::from_secs(160), async {
        let mut behind_headers = HashSet::new();
        let mut validator_headers = HashSet::new();
        loop {
            let info = behind_nodes[0].consensus_info().await;
            println!("behind node info: {info:?}");
            behind_headers.insert(info.tip);
            for v in &validators[..2] {
                let info1 = v.consensus_info().await;
                println!("validator info: {info1:?}");
                validator_headers.insert(info1.tip);
            }
            if let Some(common) = behind_headers.intersection(&validator_headers).next() {
                println!("Found common block header: {:?}", common);
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        found_common,
        "Behind node and validator should share at least one block header within 160 seconds"
    );
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
