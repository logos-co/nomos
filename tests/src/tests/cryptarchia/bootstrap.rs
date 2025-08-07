use std::{collections::HashSet, time::Duration};

use futures::stream::{self, StreamExt as _};
use nomos_libp2p::PeerId;
use tests::{
    adjust_timeout,
    common::sync::wait_for_validators_mode_and_height,
    nodes::validator::{create_validator_config, Validator},
    secret_key_to_peer_id,
    topology::configs::{create_general_configs, GeneralConfig},
};

#[tokio::test]
async fn test_ibd_behind_nodes() {
    let n_validators = 4;
    let n_initial_validators = 2;

    let general_configs = create_general_configs(n_validators);

    let mut initial_validators = vec![];
    for config in general_configs.iter().take(n_initial_validators) {
        let config = create_validator_config(config.clone());
        initial_validators.push(Validator::spawn(config).await.unwrap());
    }

    println!("Testing IBD while initial validators are still bootstrapping...");

    let initial_peer_ids: HashSet<PeerId> = general_configs
        .iter()
        .take(n_initial_validators)
        .map(|config| secret_key_to_peer_id(config.network_config.swarm_config.node_key.clone()))
        .collect();

    let mut config = create_validator_config(general_configs[n_initial_validators].clone());
    config.cryptarchia.bootstrap.ibd.peers = initial_peer_ids.clone();

    let failing_behind_node = Validator::spawn(config).await;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // IBD failed and node stopped
    assert!(failing_behind_node.is_err());

    let minimum_height = 10;
    println!("Waiting for initial validators to switch to online mode and reach height {minimum_height}...", );

    wait_for_validators_mode_and_height(
        &initial_validators,
        cryptarchia_engine::State::Online,
        minimum_height,
    )
    .await;

    println!("Starting behind node with IBD peers...");

    let mut config = create_validator_config(general_configs[3].clone());
    config.cryptarchia.bootstrap.ibd.peers = initial_peer_ids.clone();

    let behind_node = Validator::spawn(config)
        .await
        .expect("Behind node should start successfully");

    println!("Behind node started, waiting for it to sync...");

    // 1 second is enough to catch up to the initial validators
    let conservative_ibd_duration = adjust_timeout(Duration::from_secs(1));
    tokio::time::sleep(conservative_ibd_duration).await;

    // Check if the behind node has caught up to the highest initial validator.
    let heights = stream::iter(&initial_validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect::<Vec<_>>()
        .await;

    let max_initial_validator_height = heights
        .iter()
        .max()
        .expect("There should be at least one initial validator");

    let behind_node_info = behind_node.consensus_info().await;
    println!("behind node info: {behind_node_info:?}");

    let block_time = calculate_block_time(general_configs.first().unwrap());
    println!("Estimated block time: {block_time:?}");

    let conservative_height_check_duration = Duration::from_secs(1);
    let time_buffer = conservative_ibd_duration + conservative_height_check_duration;

    let height_margin = time_buffer.div_duration_f64(block_time).ceil() as u64;
    println!("Estimated height margin: {height_margin}");

    println!("Checking if the behind node has caught up to the highest initial validator");

    assert!(
        behind_node_info
            .height
            .abs_diff(*max_initial_validator_height)
            <= height_margin
    );
}

fn calculate_block_time(general_config: &GeneralConfig) -> Duration {
    let slot_duration = general_config.time_config.slot_duration;
    let active_slot_coeff = general_config
        .consensus_config
        .ledger_config
        .consensus_config
        .active_slot_coeff;
    println!("slot_duration:{slot_duration:?}, active_slot_coeff:{active_slot_coeff:?}");
    slot_duration.div_f64(active_slot_coeff)
}
