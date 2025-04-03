mod cryptarchia;

use crate::cryptarchia::{CryptarchiaSyncService, CryptarchiaSyncServiceConfig, MockBlock};
use nomos_libp2p::{ed25519, Multiaddr, SwarmConfig};
use nomos_network::{backends::libp2p::Libp2pConfig, NetworkConfig};
use nomos_node::{NetworkBackend, RocksBackend, Wire};
use nomos_storage::backends::rocksdb::RocksBackendSettings;
use overwatch::derive_services;
use overwatch::overwatch::OverwatchRunner;
use serde::{Deserialize, Serialize};

type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;
type StorageService = nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;

type Cryptarchia = CryptarchiaSyncService<RocksBackend<Wire>, MockBlock, RuntimeServiceId>;

#[derive_services]
pub struct NomosLightNode {
    network: NetworkService,
    storage: StorageService,
    cryptarchia: Cryptarchia,
}

#[test]
fn test_sync_two_nodes() {
    // Create settings for first node
    let node1_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    host: "0.0.0.0".parse().unwrap(),
                    port: 3000,
                    node_key: ed25519::SecretKey::generate(),
                    gossipsub_config: Default::default(),
                },
                initial_peers: vec![],
            },
        },
        storage: RocksBackendSettings {
            db_path: "node1_db".into(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {},
    };

    // Create settings for second node
    let node2_settings = NomosLightNodeServiceSettings {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: SwarmConfig {
                    host: "0.0.0.0".parse().unwrap(),
                    port: 3001,
                    node_key: ed25519::SecretKey::generate(),
                    gossipsub_config: Default::default(),
                },
                initial_peers: vec!["/ip4/127.0.0.1/tcp/3000".parse().unwrap()],
            },
        },
        storage: RocksBackendSettings {
            db_path: "node2_db".into(),
            read_only: false,
            column_family: None,
        },
        cryptarchia: CryptarchiaSyncServiceConfig {},
    };

    // Start first node
    let node1 = OverwatchRunner::<NomosLightNode>::run(node1_settings, None).unwrap();

    // Start second node
    let node2 = OverwatchRunner::<NomosLightNode>::run(node2_settings, None).unwrap();

    // Wait for nodes to finish
    node1.wait_finished();
    node2.wait_finished();
}
