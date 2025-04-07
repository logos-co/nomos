mod cryptarchia;

use std::num::NonZero;

use cryptarchia_engine::Config;
use nomos_core::header::HeaderId;
use nomos_libp2p::{ed25519, SwarmConfig};
use nomos_network::{backends::libp2p::Libp2pConfig, NetworkConfig};
use nomos_node::{NetworkBackend, RocksBackend, Wire};
use nomos_storage::backends::rocksdb::RocksBackendSettings;
use overwatch::{
    derive_services,
    overwatch::{Overwatch, OverwatchRunner},
};
use tempfile::Builder;
use tests::get_available_port;
use tokio::sync::oneshot::Receiver;

use crate::cryptarchia::{
    CryptarchiaSyncService, CryptarchiaSyncServiceConfig, CryptarchiaSyncServiceMessage, TestBlock,
};

type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;
type StorageService = nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;
type Cryptarchia = CryptarchiaSyncService<RuntimeServiceId>;

#[derive_services]
pub struct NomosLightNode {
    network: NetworkService,
    storage: StorageService,
    cryptarchia: Cryptarchia,
}

fn linear_chain(start: u64, end: u64) -> Vec<TestBlock> {
    (start..=end)
        .map(|id| (id, if id == 0 { None } else { Some(id - 1) }, id).into())
        .collect()
}

// 0 -> 1 -> 2 -> 5
//   -> 3 -> 4
fn fork_blocks() -> Vec<TestBlock> {
    // Header, Parent, Slot
    vec![
        (0, None, 0),    // Genesis
        (1, Some(0), 1), // Main chain
        (2, Some(1), 2),
        (3, Some(0), 1), // Fork
        (4, Some(3), 2),
        (5, Some(2), 3),
    ]
    .into_iter()
    .map(Into::into)
    .collect()
}

#[must_use]
pub fn id_from_u64(id: u64) -> HeaderId {
    let mut bytes = [0; 32];
    bytes[0..8].copy_from_slice(&id.to_le_bytes());
    bytes.into()
}

async fn validate_sync_result(
    reply_rcv: Receiver<Vec<TestBlock>>,
    mut expected_blocks: Vec<TestBlock>,
) {
    let mut blocks = reply_rcv.await.unwrap();
    blocks.sort_by(|a, b| a.id.cmp(&b.id).then(a.slot.cmp(&b.slot)));
    expected_blocks.sort_by(|a, b| a.id.cmp(&b.id).then(a.slot.cmp(&b.slot)));
    assert_eq!(blocks, expected_blocks);

    let config = Config {
        security_param: NonZero::new(1).unwrap(),
        active_slot_coeff: 1.0,
    };

    // Make sure that Cryptarchia also agrees with the blocks
    blocks.sort_by(|a, b| a.slot.cmp(&b.slot));
    let mut cryptarchia = cryptarchia_engine::Cryptarchia::new(HeaderId::from([0; 32]), config);
    for block in &blocks[1..] {
        let updated_cryptarchia =
            cryptarchia.receive_block(block.id, block.parent.unwrap(), block.slot);
        assert!(updated_cryptarchia.is_ok());
        cryptarchia = updated_cryptarchia.unwrap();
    }
}

#[test]
fn run_sync_empty_blockchain() {
    let genesis_block: Vec<_> = vec![(0, None, 0).into()];

    let (_passive_node, active_node, reply_rcv) =
        setup_nodes(&genesis_block, genesis_block.clone());

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, genesis_block));
}

#[test]
fn run_sync_single_chain_from_genesis() {
    let all_blocks = linear_chain(0, 9);
    let active_blocks = linear_chain(0, 0);
    let (_passive_node, active_node, reply_rcv) = setup_nodes(&all_blocks, active_blocks);

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, all_blocks));
}

#[test]
fn run_sync_single_chain_from_middle() {
    let all_blocks = linear_chain(0, 9);
    let active_blocks = linear_chain(0, 4);
    let (_passive_node, active_node, reply_rcv) = setup_nodes(&all_blocks, active_blocks);

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, all_blocks));
}

#[test]
fn run_sync_forks_from_genesis() {
    let all_blocks = fork_blocks();
    let active_blocks = linear_chain(0, 0);
    let (_passive_node, active_node, reply_rcv) = setup_nodes(&all_blocks, active_blocks);

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, all_blocks));
}

#[test]
fn run_sync_forks_from_middle() {
    let all_blocks = fork_blocks();
    let active_blocks = vec![(0, None, 0), (1, Some(0), 1), (3, Some(0), 1)]
        .into_iter()
        .map(Into::into)
        .collect();
    let (_passive_node, active_node, reply_rcv) = setup_nodes(&all_blocks, active_blocks);

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, all_blocks));
}

#[test]
fn run_sync_forks_by_backfilling() {
    let all_blocks = fork_blocks();
    let active_blocks = vec![(0, None, 0), (1, Some(0), 1)]
        .into_iter()
        .map(Into::into)
        .collect();
    let (_passive_node, active_node, reply_rcv) = setup_nodes(&all_blocks, active_blocks);

    let runtime = active_node.handle().runtime().clone();
    active_node.wait_finished();

    runtime.block_on(validate_sync_result(reply_rcv, all_blocks));
}

fn setup_nodes(
    all_blocks: &[TestBlock],
    behind_blocks: Vec<TestBlock>,
) -> (
    Overwatch<RuntimeServiceId>,
    Overwatch<RuntimeServiceId>,
    Receiver<Vec<TestBlock>>,
) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let passive_port = get_available_port();
    let passive_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    port: passive_port,
                    node_key: ed25519::SecretKey::generate(),
                    ..Default::default()
                },
                initial_peers: vec![],
            },
        },
        storage: RocksBackendSettings {
            db_path: Builder::new()
                .prefix("node_1")
                .tempdir()
                .unwrap()
                .path()
                .to_path_buf(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {
            topic: "topic".to_owned(),
            active: false,
            initial_blocks: all_blocks.to_owned(),
        },
    };

    let active_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    port: get_available_port(),
                    ..Default::default()
                },
                initial_peers: vec![format!("/memory/{passive_port}").parse().unwrap()],
            },
        },
        storage: RocksBackendSettings {
            db_path: Builder::new()
                .prefix("node_2")
                .tempdir()
                .unwrap()
                .path()
                .to_path_buf(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {
            topic: "topic".to_owned(),
            active: true,
            initial_blocks: behind_blocks,
        },
    };

    let passive_node = OverwatchRunner::<NomosLightNode>::run(passive_settings, None).unwrap();
    let active_node = OverwatchRunner::<NomosLightNode>::run(active_settings, None).unwrap();

    let runtime = passive_node.handle().runtime();
    let (reply_tx, reply_rcv) = tokio::sync::oneshot::channel();
    let handle = active_node.handle().clone();
    runtime.spawn(async move {
        let outbound_relay = handle.relay::<CryptarchiaSyncService<_>>().await.unwrap();
        outbound_relay
            .send(CryptarchiaSyncServiceMessage { reply_tx })
            .await
            .unwrap();
    });

    (passive_node, active_node, reply_rcv)
}
