use std::time::Duration;

use futures_util::{stream, StreamExt};
use nomos_node::Config;

use crate::{adjust_timeout, nodes::validator::Validator};

// how many times more than the expected time to produce a predefined number of
// blocks we wait before timing out
const TIMEOUT_MULTIPLIER: f64 = 10.0;
// how long we let the chain grow before checking the block at tip - k is the
// same in all chains
pub const CHAIN_LENGTH_MULTIPLIER: u32 = 2;

pub async fn wait_blocks(n_blocks: u32, nodes: &[Validator], config: &Config) {
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
}
