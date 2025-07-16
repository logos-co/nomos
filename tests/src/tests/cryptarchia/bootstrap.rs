use std::{collections::HashSet, time::Duration};

use futures::stream::{self, StreamExt as _};
use nomos_libp2p::PeerId;
use tests::{
    adjust_timeout,
    nodes::validator::{create_validator_config, Validator},
    secret_key_to_peer_id,
    topology::configs::create_general_configs,
};

const CHAIN_LENGTH_MULTIPLIER: u32 = 2;

#[tokio::test]
async fn test_ibd_behind_nodes() {
    let n_validators = 4;

    let general_configs = create_general_configs(n_validators);
    let consensus_params = general_configs[0]
        .consensus_config
        .ledger_config
        .consensus_config;

    let mut validators = vec![];
    for config in general_configs.iter().take(2) {
        let config = create_validator_config(config.clone());
        validators.push(Validator::spawn(config).await.unwrap());
    }

    let security_param = consensus_params.security_param.get();
    let target_blocks = security_param * CHAIN_LENGTH_MULTIPLIER;

    let min_blocks = 5;

    println!(
        "Waiting for initial validators to reach at least min_blocks ({min_blocks}) but still be in bootstrapping mode...",
    );

    wait_for_validators_mode_and_height(
        &validators,
        min_blocks,
        cryptarchia_engine::State::Bootstrapping,
    )
    .await;

    let initial_peer_ids: HashSet<PeerId> = general_configs
        .iter()
        .take(2)
        .map(|config| secret_key_to_peer_id(config.network_config.swarm_config.node_key.clone()))
        .collect();

    println!("Testing IBD while initial validators are still bootstrapping...");

    let mut config = create_validator_config(general_configs[2].clone());
    config.cryptarchia.bootstrap.ibd.peers = initial_peer_ids.clone();

    let failing_behind_node = Validator::spawn(config).await;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // IBD failed and node stopped
    assert!(failing_behind_node.is_err());

    println!(
        "Waiting for initial validators to reach target_blocks ({target_blocks}) and switch to online mode...",
    );

    wait_for_validators_mode_and_height(
        &validators,
        target_blocks,
        cryptarchia_engine::State::Online,
    )
    .await;

    println!("Starting behind node with IBD peers...");

    let mut config = create_validator_config(general_configs[3].clone());
    config.cryptarchia.bootstrap.ibd.peers = initial_peer_ids.clone();

    let behind_node = Validator::spawn(config)
        .await
        .expect("Behind node should start successfully");

    println!("Behind node started, waiting for it to sync...");

    let initial_heights: Vec<_> = stream::iter(&validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect()
        .await;

    let initial_node_min_height = initial_heights.iter().min().unwrap();

    // 1 second is enough to download the initial blocks and catch up
    tokio::time::sleep(adjust_timeout(Duration::from_secs(1))).await;

    let behind_node_info = behind_node.consensus_info().await;
    println!("behind node info: {behind_node_info:?}");

    assert!(behind_node_info.height >= *initial_node_min_height);
}

async fn wait_for_validators_mode_and_height(
    validators: &[Validator],
    min_height: u32,
    mode: cryptarchia_engine::State,
) {
    loop {
        let infos: Vec<_> = stream::iter(validators)
            .then(|n| async move { n.consensus_info().await })
            .collect()
            .await;

        println!(
            "   Initial validators: [{}]",
            infos
                .iter()
                .map(|info| format!("{:?}/{:?}", info.height, info.mode))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let current_min_height = infos.iter().map(|info| info.height).min().unwrap_or(0) as u32;

        if current_min_height >= min_height && infos.iter().all(|info| info.mode == mode) {
            println!("   All validators reached min height {min_height} and are in mode {mode:?}",);
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
