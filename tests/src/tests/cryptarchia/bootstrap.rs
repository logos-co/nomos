use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use futures::stream::{self, StreamExt as _};
use nomos_libp2p::PeerId;
use tests::{
    common::sync::{wait_for_validators_mode, wait_for_validators_mode_and_height},
    nodes::validator::{create_validator_config, Validator},
    secret_key_to_peer_id,
    topology::configs::{
        create_general_configs_with_network,
        network::{Libp2pNetworkLayout, NetworkParams},
        GeneralConfig,
    },
};

#[tokio::test]
async fn test_ibd_behind_nodes() {
    let n_validators = 4;
    let n_initial_validators = 2;

    let network_params = NetworkParams {
        libp2p_network_layout: Libp2pNetworkLayout::Full,
    };
    let general_configs = create_general_configs_with_network(n_validators, &network_params);

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

    // IBD failed and node stopped
    assert!(failing_behind_node.is_err());
    // Kill the node process.
    drop(failing_behind_node);

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
    // Shorten the delay to quickly catching up with peers that grow during IBD.
    // e.g. We start a download only for peer1 because two peers have the same tip
    //      at the moment. But, the peer2 may grow faster than peer1 before IBD is
    // done.      So, we want to check peer1's progress frequently with a very
    // short delay.
    config.cryptarchia.bootstrap.ibd.delay_before_new_download = Duration::from_millis(10);
    // Disable the prolonged bootstrap period for the behind node
    // because we want to check the height of the behind node
    // as soon as it finishes IBD.
    // Currently, checking the mode is only one way to check if IBD is done.
    config.cryptarchia.bootstrap.prolonged_bootstrap_period = Duration::ZERO;

    let behind_node = Validator::spawn(config)
        .await
        .expect("Behind node should start successfully");

    println!("Behind node started, waiting for it to finish IBD and switch to online mode...");
    wait_for_validators_mode(&[&behind_node], cryptarchia_engine::State::Online).await;

    // Check if the behind node has caught up to the highest initial validator.
    let height_check_timestamp = Instant::now();
    let heights = stream::iter(&initial_validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect::<Vec<_>>()
        .await;
    println!("initial validator heights: {heights:?}");

    let max_initial_validator_height = heights
        .iter()
        .max()
        .expect("There should be at least one initial validator");

    let behind_node_info = behind_node.consensus_info().await;
    println!("behind node info: {behind_node_info:?}");

    // We spent some time for checking the heights of nodes
    // after the behind node finishes IBD.
    // So, calculate an acceptable height margin for safe comparison.
    let height_margin = acceptable_height_margin(
        general_configs.first().unwrap(),
        height_check_timestamp.elapsed(),
    );

    println!("Checking if the behind node has caught up to the highest initial validator");
    assert!(
        behind_node_info
            .height
            .abs_diff(*max_initial_validator_height)
            <= height_margin,
    );
}

fn acceptable_height_margin(general_config: &GeneralConfig, duration: Duration) -> u64 {
    let block_time = calculate_block_time(general_config);
    let margin = duration.div_duration_f64(block_time).ceil() as u64;
    println!("Acceptable height margin:{margin} for duration {duration:?} with block time {block_time:?}");
    margin
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
