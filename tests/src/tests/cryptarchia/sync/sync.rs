mod cryptarchia;

use crate::cryptarchia::{CryptarchiaSyncService, CryptarchiaSyncServiceConfig};
use nomos_libp2p::{ed25519, gossipsub, SwarmConfig};
use nomos_network::{backends::libp2p::Libp2pConfig, NetworkConfig};
use nomos_node::{NetworkBackend, RocksBackend, Wire};
use nomos_storage::backends::rocksdb::RocksBackendSettings;
use overwatch::derive_services;
use overwatch::overwatch::OverwatchRunner;

type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;
type StorageService = nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;
type Cryptarchia = CryptarchiaSyncService<RuntimeServiceId>;

#[derive_services]
pub struct NomosLightNode {
    network: NetworkService,
    storage: StorageService,
    cryptarchia: Cryptarchia,
}

#[test]
fn test_sync_two_nodes() {
    tracing_subscriber::fmt::init();
    let node1_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    host: "0.0.0.0".parse().unwrap(),
                    port: 3000,
                    node_key: ed25519::SecretKey::generate(),
                    gossipsub_config: gossipsub::Config::default(),
                },
                initial_peers: vec!["/memory/3001".parse().unwrap()],
            },
        },
        storage: RocksBackendSettings {
            db_path: "node3_db".into(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {
            topic: "topic".to_owned(),
        },
    };

    // Create settings for second node
    let node2_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    host: "0.0.0.0".parse().unwrap(),
                    port: 3001,
                    node_key: ed25519::SecretKey::generate(),
                    gossipsub_config: gossipsub::Config::default(),
                },
                initial_peers: vec!["/memory/3000".parse().unwrap()],
            },
        },
        storage: RocksBackendSettings {
            db_path: "node4_db".into(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {
            topic: "topic".to_owned(),
        },
    };

    let node1 = OverwatchRunner::<NomosLightNode>::run(node1_settings, None).unwrap();

    let node2 = OverwatchRunner::<NomosLightNode>::run(node2_settings, None).unwrap();

    node1.wait_finished();
    node2.wait_finished();
}
