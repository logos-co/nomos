use std::{collections::HashSet, time::Duration};

use futures::stream::{self, StreamExt as _};
use serial_test::serial;
use tests::{
    adjust_timeout,
    topology::{Topology, TopologyConfig},
};

// how many times more than the expected time to produce a predefined number of
// blocks we wait before timing out
const TIMEOUT_MULTIPLIER: f64 = 3.0;
// how long we let the chain grow before checking the block at tip - k is the
// same in all chains
const CHAIN_LENGTH_MULTIPLIER: u32 = 2;

async fn happy_test(topology: &Topology) {
    let nodes = topology.validators();
    let config = nodes[0].config();
    let security_param = config.cryptarchia.config.consensus_config.security_param;
    let n_blocks = security_param.get() * CHAIN_LENGTH_MULTIPLIER;
    println!("waiting for {n_blocks} blocks");
    let timeout = (f64::from(n_blocks)
        / config.cryptarchia.config.consensus_config.active_slot_coeff
        * config
            .time
            .backend_settings
            .slot_config
            .slot_duration
            .as_secs() as f64
        * TIMEOUT_MULTIPLIER)
        .floor() as u64;
    let timeout = adjust_timeout(Duration::from_secs(timeout));
    let timeout = tokio::time::sleep(timeout);
    {
        tokio::select! {
            () = timeout => panic!("timed out waiting for nodes to produce {} blocks", n_blocks),
            () = async { while stream::iter(nodes)
                .any(|n| async move { (n.consensus_info().await.height as u32) < n_blocks })
                .await
            {
                println!(
                    "waiting... {}",
                    stream::iter(nodes)
                        .then(|n| async move { format!("{}", n.consensus_info().await.height) })
                        .collect::<Vec<_>>()
                        .await
                        .join(" | ")
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            } => {}
        }
    }

    println!("{:?}", nodes[0].consensus_info().await);

    let infos = stream::iter(nodes)
        .then(|n| async move { n.get_headers(None, None).await })
        // TODO: this can actually fail if the one node is slightly behind, we should really either
        // get the block at a specific height, but we currently lack the API for that
        .map(|blocks| blocks.last().copied().unwrap()) // we're getting the LIB
        .collect::<HashSet<_>>()
        .await;

    assert_eq!(infos.len(), 1, "consensus not reached");
}

#[tokio::test]
#[serial]
async fn two_nodes_happy() {
    let topology = Topology::spawn(TopologyConfig::two_validators()).await;
    happy_test(&topology).await;
}
