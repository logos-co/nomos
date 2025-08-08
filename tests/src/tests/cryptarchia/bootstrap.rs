use std::{collections::HashSet, ops::Div as _, time::Duration};

use futures::stream::{self, StreamExt as _};
use nomos_libp2p::PeerId;
use tests::{
    adjust_timeout,
    nodes::validator::{create_validator_config, Validator},
    secret_key_to_peer_id,
    topology::configs::{create_general_configs, GeneralConfig},
};

#[tokio::test]
async fn test_ibd_behind_nodes() {
    let n_validators = 4;
    let n_initial_validators = 2;

    let general_configs = create_general_configs(n_validators);

    let mut validators = vec![];
    for config in general_configs.iter().take(n_initial_validators) {
        let config = create_validator_config(config.clone());
        validators.push(Validator::spawn(config).await.unwrap());
    }
    let block_time = calculate_block_time(general_configs.first().unwrap());

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

    println!("Waiting for initial validators to switch to online mode...",);

    wait_for_validators_mode_and_height(&validators, cryptarchia_engine::State::Online, 10).await;

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
    //
    // Keep in mind that any validator could have produced new blocks,
    // and they have been not yet propagated to the network, before we perform this check.
    // Even the behind node could have produced blocks as soon as IBD was completed.
    // It means that the behind node could have the chain higher than others.
    let max_initial_validator_height = *stream::iter(&validators)
        .take(n_initial_validators)
        .then(|n| async move { n.consensus_info().await.height })
        .collect::<Vec<_>>()
        .await
        .iter()
        .max()
        .unwrap();
    let behind_node_info = behind_node.consensus_info().await;
    println!("behind node info: {behind_node_info:?}");
    let conservative_height_check_duration = Duration::from_secs(1);

    let time_buffer = conservative_ibd_duration + conservative_height_check_duration;
    let height_margin = time_buffer.div_duration_f64(block_time).ceil() as u64;
    println!("Checking if the behind node has caught up to the highest initial validator: height_margin:{height_margin}");
    assert!(
        behind_node_info
            .height
            .abs_diff(max_initial_validator_height)
            <= height_margin
    );
}

fn calculate_block_time(general_config: &GeneralConfig) -> Duration {
    Duration::from_secs(
        1f64.div(
            general_config
                .consensus_config
                .ledger_config
                .consensus_config
                .active_slot_coeff,
        )
        .floor() as u64,
    )
}

async fn wait_for_validators_mode_and_height(
    validators: &[Validator],
    mode: cryptarchia_engine::State,
    min_height: u64,
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

        if infos.iter().all(|info| info.mode == mode)
            && infos.iter().all(|info| info.height >= min_height)
        {
            println!("   All validators reached are in mode {mode:?}",);
            break;
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}
