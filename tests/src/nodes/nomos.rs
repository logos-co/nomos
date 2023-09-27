// std
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
// internal
use crate::{get_available_port, Node, SpawnConfig};
use consensus_engine::overlay::{RandomBeaconState, RoundRobin, TreeOverlay, TreeOverlaySettings};
use consensus_engine::{NodeId, Overlay};
use mixnet_client::{MixnetClientConfig, MixnetClientMode};
use mixnet_node::MixnetNodeConfig;
use mixnet_topology::MixnetTopology;
use nomos_consensus::{CarnotInfo, CarnotSettings};
use nomos_http::backends::axum::AxumBackendSettings;
use nomos_libp2p::Multiaddr;
use nomos_log::{LoggerBackend, LoggerFormat};
use nomos_network::backends::libp2p::{Libp2pConfig, Libp2pInfo};
#[cfg(feature = "waku")]
use nomos_network::backends::waku::{WakuConfig, WakuInfo};
use nomos_network::NetworkConfig;
use nomos_node::Config;
// crates
use fraction::Fraction;
use once_cell::sync::Lazy;
use rand::{thread_rng, Rng};
use reqwest::Client;
use tempfile::NamedTempFile;

static CLIENT: Lazy<Client> = Lazy::new(Client::new);
const NOMOS_BIN: &str = "../target/debug/nomos-node";
const CARNOT_INFO_API: &str = "carnot/info";
const NETWORK_INFO_API: &str = "network/info";
const LOGS_PREFIX: &str = "__logs";

pub struct NomosNode {
    addr: SocketAddr,
    _tempdir: tempfile::TempDir,
    child: Child,
    config: Config,
}

impl Drop for NomosNode {
    fn drop(&mut self) {
        if std::thread::panicking() {
            println!("persisting directory at {}", self._tempdir.path().display());
            // we need ownership of the dir to persist it
            let dir = std::mem::replace(&mut self._tempdir, tempfile::tempdir().unwrap());
            // a bit confusing but `into_path` persists the directory
            let _ = dir.into_path();
        }

        self.child.kill().unwrap();
    }
}

impl NomosNode {
    pub async fn spawn(mut config: Config) -> Self {
        // Waku stores the messages in a db file in the current dir, we need a different
        // directory for each node to avoid conflicts
        let dir = tempfile::tempdir().unwrap();
        let mut file = NamedTempFile::new().unwrap();
        let config_path = file.path().to_owned();

        // setup logging so that we can intercept it later in testing
        config.log.backend = LoggerBackend::File {
            directory: dir.path().to_owned(),
            prefix: Some(LOGS_PREFIX.into()),
        };
        config.log.format = LoggerFormat::Json;

        serde_yaml::to_writer(&mut file, &config).unwrap();
        let child = Command::new(std::env::current_dir().unwrap().join(NOMOS_BIN))
            .arg(&config_path)
            .current_dir(dir.path())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let node = Self {
            addr: config.http.backend.address,
            child,
            _tempdir: dir,
            config,
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            node.wait_online().await
        })
        .await
        .unwrap();

        node
    }

    async fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        CLIENT
            .get(format!("http://{}/{}", self.addr, path))
            .send()
            .await
    }

    async fn wait_online(&self) {
        while self.get(CARNOT_INFO_API).await.is_err() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    pub async fn get_listening_address(&self) -> Multiaddr {
        self.get(NETWORK_INFO_API)
            .await
            .unwrap()
            .json::<Libp2pInfo>()
            .await
            .unwrap()
            .listen_addresses
            .swap_remove(0)
    }

    // not async so that we can use this in `Drop`
    pub fn get_logs_from_file(&self) -> String {
        println!(
            "fetching logs from dir {}...",
            self._tempdir.path().display()
        );
        // std::thread::sleep(std::time::Duration::from_secs(50));
        std::fs::read_dir(self._tempdir.path())
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_file() && path.to_str().unwrap().contains(LOGS_PREFIX) {
                    Some(path)
                } else {
                    None
                }
            })
            .map(|f| std::fs::read_to_string(f).unwrap())
            .collect::<String>()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[async_trait::async_trait]
impl Node for NomosNode {
    type ConsensusInfo = CarnotInfo;

    async fn spawn_nodes(config: SpawnConfig) -> Vec<Self> {
        match config {
            SpawnConfig::Star {
                n_participants,
                threshold,
                timeout,
                mut mixnet_node_configs,
                mixnet_topology,
            } => {
                let mut ids = vec![[0; 32]; n_participants];
                for id in &mut ids {
                    thread_rng().fill(id);
                }

                let mut configs = ids
                    .iter()
                    .map(|id| {
                        create_node_config(
                            ids.iter().copied().map(NodeId::new).collect(),
                            *id,
                            threshold,
                            timeout,
                            mixnet_node_configs.pop(),
                            mixnet_topology.clone(),
                        )
                    })
                    .collect::<Vec<_>>();

                let overlay = TreeOverlay::new(configs[0].consensus.overlay_settings.clone());
                let next_leader = overlay.next_leader();
                let next_leader_idx = ids
                    .iter()
                    .position(|&id| NodeId::from(id) == next_leader)
                    .unwrap();

                // Spawn the next leader first, so the leader can receive votes from
                // all nodes that will be subsequently spawned.
                // If not, the leader will miss votes from nodes spawned before itself.
                // This issue will be resolved by devising the block catch-up mechanism in the future
                let mut nodes = vec![Self::spawn(configs.swap_remove(next_leader_idx)).await];
                let listening_addr = nodes[0].get_listening_address().await;
                for mut conf in configs {
                    conf.network
                        .backend
                        .initial_peers
                        .push(listening_addr.clone());

                    nodes.push(Self::spawn(conf).await);
                }
                nodes
            }
        }
    }

    async fn consensus_info(&self) -> Self::ConsensusInfo {
        self.get(CARNOT_INFO_API)
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    fn stop(&mut self) {
        self.child.kill().unwrap();
    }
}

fn create_node_config(
    nodes: Vec<NodeId>,
    private_key: [u8; 32],
    _threshold: Fraction,
    timeout: Duration,
    mixnet_node_config: Option<MixnetNodeConfig>,
    mixnet_topology: MixnetTopology,
) -> Config {
    let mixnet_client_mode = match mixnet_node_config {
        Some(node_config) => MixnetClientMode::SenderReceiver(node_config.client_listen_address),
        None => MixnetClientMode::Sender,
    };

    let mut config = Config {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: Default::default(),
                initial_peers: vec![],
                mixnet_client: MixnetClientConfig {
                    mode: mixnet_client_mode,
                    topology: mixnet_topology,
                    connection_pool_size: 255,
                    max_retries: 3,
                    retry_delay: Duration::from_secs(5),
                },
                mixnet_delay: Duration::ZERO..Duration::from_millis(10),
            },
        },
        consensus: CarnotSettings {
            private_key,
            overlay_settings: TreeOverlaySettings {
                nodes,
                leader: RoundRobin::new(),
                current_leader: [0; 32].into(),
                number_of_committees: 1,
                committee_membership: RandomBeaconState::initial_sad_from_entropy([0; 32]),
            },
            timeout,
            transaction_selector_settings: (),
            blob_selector_settings: (),
        },
        log: Default::default(),
        http: nomos_http::http::HttpServiceSettings {
            backend: AxumBackendSettings {
                address: format!("127.0.0.1:{}", get_available_port())
                    .parse()
                    .unwrap(),
                cors_origins: vec![],
            },
        },
        #[cfg(feature = "metrics")]
        metrics: Default::default(),
        da: nomos_da::Settings {
            da_protocol: full_replication::Settings {
                num_attestations: 1,
            },
            backend: nomos_da::backend::memory_cache::BlobCacheSettings {
                max_capacity: usize::MAX,
                evicting_period: Duration::from_secs(60 * 60 * 24), // 1 day
            },
        },
    };

    config.network.backend.inner.port = get_available_port();

    config
}
