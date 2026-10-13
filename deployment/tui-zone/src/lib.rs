mod message;
mod state;
mod ui;

use std::{fs, path::Path};

use clap::Parser;
use lb_core::mantle::{
    gas::GasCost,
    ops::channel::{ChannelId, inscribe::Inscription},
};
use lb_key_management_system_service::keys::{ED25519_SECRET_KEY_SIZE, Ed25519Key, ZkPublicKey};
use lb_zone_sdk::{
    CommonHttpClient,
    adapter::NodeHttpClient,
    sequencer::{Event, FinalizedOp, FinalizedTx, FundingConfig, InscriptionInfo, ZoneSequencer},
};
use reqwest::Url;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    message::AppMessage,
    state::{InMemoryZoneState, ZoneState as _},
};

#[derive(Parser, Debug)]
#[command(about = "Terminal UI zone sequencer - publish text inscriptions")]
pub struct InscribeArgs {
    /// Logos blockchain node HTTP endpoint
    #[arg(long, default_value = "http://localhost:8080", env = "NODE_URL")]
    node_url: String,

    /// Path to the signing key file (created if it doesn't exist)
    #[arg(long, default_value = "sequencer.key", env = "KEY_PATH")]
    key_path: String,

    /// Hex-encoded ZK public key of a funded key in the node's wallet. The
    /// node spends this key's notes to pay each inscription's transaction fee.
    #[arg(long, env = "FUNDING_PK", value_parser = parse_zk_public_key)]
    funding_pk: ZkPublicKey,
}

fn parse_zk_public_key(hex_key: &str) -> Result<ZkPublicKey, String> {
    serde_json::from_value(serde_json::Value::String(hex_key.to_owned()))
        .map_err(|e| format!("invalid ZK public key: {e}"))
}

fn spawn_stdin_reader(ready: tokio::sync::oneshot::Receiver<()>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(16);
    std::thread::spawn(move || {
        // Wait until the sequencer is ready before accepting input
        if ready.blocking_recv().is_err() {
            return;
        }

        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = line.trim_end().to_owned();
                    if text.is_empty() || tx.blocking_send(text).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

// Your Code Here
