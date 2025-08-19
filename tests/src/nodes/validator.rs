use std::{
    collections::HashSet,
    net::SocketAddr,
    num::{NonZeroU64, NonZeroUsize},
    ops::Range,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use chain_service::{CryptarchiaInfo, CryptarchiaSettings, OrphanConfig, SyncConfig};
use cryptarchia_engine::time::SlotConfig;
use kzgrs_backend::common::share::DaShare;
use nomos_api::http::membership::MembershipUpdateRequest;
use nomos_blend_scheduling::message_blend::CryptographicProcessorSettings;
use nomos_blend_service::{
    core::settings::{CoverTrafficSettingsExt, MessageDelayerSettingsExt, SchedulerSettingsExt},
    settings::TimingSettings,
};
use nomos_core::{block::Block, mantle::SignedMantleTx, sdp::FinalizedBlockEvent};
use nomos_da_indexer::{
    storage::adapters::rocksdb::RocksAdapterSettings as IndexerStorageAdapterSettings,
    IndexerSettings,
};
use nomos_da_network_core::{
    protocols::sampling::SubnetsConfig,
    swarm::{BalancerStats, DAConnectionPolicySettings, MonitorStats},
};
use nomos_da_network_service::{
    api::http::ApiAdapterSettings, backends::libp2p::common::DaNetworkBackendSettings,
    NetworkConfig as DaNetworkConfig,
};
use nomos_da_sampling::{backend::kzgrs::KzgrsSamplingBackendSettings, DaSamplingServiceSettings};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifierSettings,
    storage::adapters::rocksdb::RocksAdapterSettings as VerifierStorageAdapterSettings,
    DaVerifierServiceSettings,
};
use nomos_http_api_common::paths::{
    CL_METRICS, CRYPTARCHIA_HEADERS, CRYPTARCHIA_INFO, DA_BALANCER_STATS, DA_GET_RANGE,
    DA_MONITOR_STATS, STORAGE_BLOCK, UPDATE_MEMBERSHIP,
};
use nomos_mempool::MempoolMetrics;
use nomos_network::{backends::libp2p::Libp2pConfig, config::NetworkConfig};
use nomos_node::{
    api::backend::AxumBackendSettings,
    config::{blend::BlendConfig, mempool::MempoolConfig},
    BlobInfo, Config, HeaderId, RocksBackendSettings,
};
use nomos_time::{
    backends::{ntp::async_client::NTPClientSettings, NtpTimeBackendSettings},
    TimeServiceSettings,
};
use nomos_tracing::logging::local::FileConfig;
use nomos_tracing_service::LoggerLayer;
use nomos_utils::math::NonNegativeF64;
use reqwest::Url;
use tempfile::NamedTempFile;
use tokio::time::error::Elapsed;

use super::{create_tempdir, persist_tempdir, GetRangeReq, CLIENT};
use crate::{
    adjust_timeout, get_available_port, nodes::LOGS_PREFIX, topology::configs::GeneralConfig,
    IS_DEBUG_TRACING,
};

const BIN_PATH: &str = "../target/debug/nomos-node";

pub enum Pool {
    Da,
    Cl,
}

pub struct Validator {
    addr: SocketAddr,
    testing_http_addr: SocketAddr,
    tempdir: tempfile::TempDir,
    child: Child,
    config: Config,
}

impl Drop for Validator {
    fn drop(&mut self) {
        if std::thread::panicking() {
            if let Err(e) = persist_tempdir(&mut self.tempdir, "nomos-node") {
                println!("failed to persist tempdir: {e}");
            }
        }

        if let Err(e) = self.child.kill() {
            println!("failed to kill the child process: {e}");
        }
    }
}

impl Validator {
    pub async fn spawn(mut config: Config) -> Result<Self, Elapsed> {
        let dir = create_tempdir().unwrap();
        let mut file = NamedTempFile::new().unwrap();
        let config_path = file.path().to_owned();

        if !*IS_DEBUG_TRACING {
            // setup logging so that we can intercept it later in testing
            config.tracing.logger = LoggerLayer::File(FileConfig {
                directory: dir.path().to_owned(),
                prefix: Some(LOGS_PREFIX.into()),
            });
        }

        config.storage.db_path = dir.path().join("db");
        dir.path().clone_into(
            &mut config
                .da_verifier
                .storage_adapter_settings
                .blob_storage_directory,
        );
        dir.path()
            .clone_into(&mut config.da_indexer.storage.blob_storage_directory);

        serde_yaml::to_writer(&mut file, &config).unwrap();
        let child = Command::new(std::env::current_dir().unwrap().join(BIN_PATH))
            .arg(&config_path)
            .current_dir(dir.path())
            .stdout(Stdio::inherit())
            .spawn()
            .unwrap();
        let node = Self {
            addr: config.http.backend_settings.address,
            testing_http_addr: config.testing_http.backend_settings.address,
            child,
            tempdir: dir,
            config,
        };

        tokio::time::timeout(adjust_timeout(Duration::from_secs(10)), async {
            node.wait_online().await;
        })
        .await?;

        Ok(node)
    }

    async fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        CLIENT
            .get(format!("http://{}{}", self.addr, path))
            .send()
            .await
    }

    #[must_use]
    pub fn url(&self) -> Url {
        format!("http://{}", self.addr).parse().unwrap()
    }

    async fn wait_online(&self) {
        loop {
            let res = self.get(CL_METRICS).await;
            if res.is_ok() && res.unwrap().status().is_success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn get_block(&self, id: HeaderId) -> Option<Block<SignedMantleTx, BlobInfo>> {
        CLIENT
            .post(format!("http://{}{}", self.addr, STORAGE_BLOCK))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&id).unwrap())
            .send()
            .await
            .unwrap()
            .json::<Option<Block<SignedMantleTx, BlobInfo>>>()
            .await
            .unwrap()
    }

    pub async fn get_mempoool_metrics(&self, pool: Pool) -> MempoolMetrics {
        let discr = match pool {
            Pool::Cl => "cl",
            Pool::Da => "da",
        };
        let addr = format!("/{discr}/metrics");
        let res = self
            .get(&addr)
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        MempoolMetrics {
            pending_items: res["pending_items"].as_u64().unwrap() as usize,
            last_item_timestamp: res["last_item_timestamp"].as_u64().unwrap(),
        }
    }

    pub async fn get_indexer_range(
        &self,
        app_id: [u8; 32],
        range: Range<[u8; 8]>,
    ) -> Vec<([u8; 8], Vec<DaShare>)> {
        CLIENT
            .post(format!("http://{}{}", self.addr, DA_GET_RANGE))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&GetRangeReq { app_id, range }).unwrap())
            .send()
            .await
            .unwrap()
            .json::<Vec<([u8; 8], Vec<DaShare>)>>()
            .await
            .unwrap()
    }

    pub async fn update_membership(
        &self,
        update_event: FinalizedBlockEvent,
    ) -> Result<(), reqwest::Error> {
        let update_event = MembershipUpdateRequest { update_event };
        let json_body = serde_json::to_string(&update_event).unwrap();

        let response = CLIENT
            .post(format!(
                "http://{}{}",
                self.testing_http_addr, UPDATE_MEMBERSHIP
            ))
            .header("Content-Type", "application/json")
            .body(json_body)
            .send()
            .await;

        assert!(
            response.is_ok(),
            "Failed to connect to testing endpoint {}.\n\
            The binary was likely built without the 'testing' feature.\n\
            Try: cargo build --workspace --all-features",
            self.testing_http_addr
        );

        let response = response.unwrap();
        response.error_for_status()?;
        Ok(())
    }

    // not async so that we can use this in `Drop`
    #[must_use]
    pub fn get_logs_from_file(&self) -> String {
        println!(
            "fetching logs from dir {}...",
            self.tempdir.path().display()
        );
        // std::thread::sleep(std::time::Duration::from_secs(50));
        std::fs::read_dir(self.tempdir.path())
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                (path.is_file() && path.to_str().unwrap().contains(LOGS_PREFIX)).then_some(path)
            })
            .map(|f| std::fs::read_to_string(f).unwrap())
            .collect::<String>()
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    pub async fn get_headers(&self, from: Option<HeaderId>, to: Option<HeaderId>) -> Vec<HeaderId> {
        let mut req = CLIENT.get(format!("http://{}{}", self.addr, CRYPTARCHIA_HEADERS));

        if let Some(from) = from {
            req = req.query(&[("from", from)]);
        }

        if let Some(to) = to {
            req = req.query(&[("to", to)]);
        }

        let res = req.send().await;

        println!("res: {res:?}");

        res.unwrap().json::<Vec<HeaderId>>().await.unwrap()
    }

    pub async fn consensus_info(&self) -> CryptarchiaInfo {
        let res = self.get(CRYPTARCHIA_INFO).await;
        println!("{res:?}");
        res.unwrap().json().await.unwrap()
    }

    pub async fn balancer_stats(&self) -> BalancerStats {
        self.get(DA_BALANCER_STATS)
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    pub async fn monitor_stats(&self) -> MonitorStats {
        self.get(DA_MONITOR_STATS)
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

#[must_use]
#[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
pub fn create_validator_config(config: GeneralConfig) -> Config {
    let testing_http_address = format!("127.0.0.1:{}", get_available_port())
        .parse()
        .unwrap();

    let da_policy_settings = config.da_config.policy_settings;
    Config {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: config.network_config.swarm_config,
                initial_peers: config.network_config.initial_peers,
            },
        },
        blend: BlendConfig::new(nomos_blend_service::core::settings::BlendConfig {
            backend: config.blend_config.backend,
            crypto: CryptographicProcessorSettings {
                signing_private_key: config.blend_config.private_key.clone(),
                num_blend_layers: 1,
            },
            time: TimingSettings {
                round_duration: Duration::from_secs(1),
                rounds_per_interval: NonZeroU64::try_from(30u64)
                    .expect("Rounds per interval cannot be zero."),
                // (21,600 blocks * 30s per block) / 1s per round = 648,000 rounds
                rounds_per_session: NonZeroU64::try_from(648_000u64)
                    .expect("Rounds per session cannot be zero."),
                rounds_per_observation_window: NonZeroU64::try_from(30u64)
                    .expect("Rounds per observation window cannot be zero."),
            },
            scheduler: SchedulerSettingsExt {
                cover: CoverTrafficSettingsExt {
                    intervals_for_safety_buffer: 100,
                    message_frequency_per_round: NonNegativeF64::try_from(1f64)
                        .expect("Message frequency per round cannot be negative."),
                    redundancy_parameter: 0,
                },
                delayer: MessageDelayerSettingsExt {
                    maximum_release_delay_in_rounds: NonZeroU64::try_from(3u64)
                        .expect("Maximum release delay between rounds cannot be zero."),
                },
            },
            membership: config.blend_config.membership,
        }),
        cryptarchia: CryptarchiaSettings {
            leader_config: config.consensus_config.leader_config,
            config: config.consensus_config.ledger_config,
            genesis_id: HeaderId::from([0; 32]),
            genesis_state: config.consensus_config.genesis_state,
            transaction_selector_settings: (),
            blob_selector_settings: (),
            network_adapter_settings:
                chain_service::network::adapters::libp2p::LibP2pAdapterSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
            blend_broadcast_settings:
                nomos_blend_service::core::network::libp2p::Libp2pBroadcastSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
            recovery_file: PathBuf::from("./recovery/cryptarchia.json"),
            bootstrap: chain_service::BootstrapConfig {
                prolonged_bootstrap_period: config.bootstrapping_config.prolonged_bootstrap_period,
                force_bootstrap: false,
                offline_grace_period: chain_service::OfflineGracePeriodConfig {
                    grace_period: Duration::from_secs(20 * 60),
                    state_recording_interval: Duration::from_secs(60),
                },
                ibd: chain_service::IbdConfig {
                    peers: HashSet::new(),
                    delay_before_new_download: Duration::from_secs(10),
                },
            },
            sync: SyncConfig {
                orphan: OrphanConfig {
                    max_orphan_cache_size: NonZeroUsize::new(5)
                        .expect("Max orphan cache size must be non-zero"),
                },
            },
        },
        da_network: DaNetworkConfig {
            backend: DaNetworkBackendSettings {
                node_key: config.da_config.node_key,
                listening_address: config.da_config.listening_address,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: 0,
                    min_replication_peers: da_policy_settings.min_replication_peers,
                    max_dispersal_failures: da_policy_settings.max_dispersal_failures,
                    max_sampling_failures: da_policy_settings.max_sampling_failures,
                    max_replication_failures: da_policy_settings.max_replication_failures,
                    malicious_threshold: da_policy_settings.malicious_threshold,
                },
                monitor_settings: config.da_config.monitor_settings,
                balancer_interval: config.da_config.balancer_interval,
                redial_cooldown: config.da_config.redial_cooldown,
                replication_settings: config.da_config.replication_settings,
                subnets_settings: SubnetsConfig {
                    num_of_subnets: config.da_config.num_samples as usize,
                    shares_retry_limit: config.da_config.retry_shares_limit,
                    commitments_retry_limit: config.da_config.retry_commitments_limit,
                },
                refresh_interval: config.da_config.subnets_refresh_interval,
            },
            membership: config.da_config.membership.clone(),
            api_adapter_settings: ApiAdapterSettings {
                api_port: config.api_config.address.port(),
                is_secure: false,
            },
        },
        da_indexer: IndexerSettings {
            storage: IndexerStorageAdapterSettings {
                blob_storage_directory: "./".into(),
            },
        },
        da_verifier: DaVerifierServiceSettings {
            verifier_settings: KzgrsDaVerifierSettings {
                global_params_path: config.da_config.global_params_path,
                domain_size: config.da_config.num_subnets as usize,
            },
            network_adapter_settings: (),
            storage_adapter_settings: VerifierStorageAdapterSettings {
                blob_storage_directory: "./".into(),
            },
        },
        tracing: config.tracing_config.tracing_settings,
        http: nomos_api::ApiServiceSettings {
            backend_settings: AxumBackendSettings {
                address: config.api_config.address,
                cors_origins: vec![],
            },
            request_timeout: None,
        },
        da_sampling: DaSamplingServiceSettings {
            sampling_settings: KzgrsSamplingBackendSettings {
                num_samples: config.da_config.num_samples,
                num_subnets: config.da_config.num_subnets,
                old_blobs_check_interval: config.da_config.old_blobs_check_interval,
                blobs_validity_duration: config.da_config.blobs_validity_duration,
            },
        },
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        // TODO from
        time: TimeServiceSettings {
            backend_settings: NtpTimeBackendSettings {
                ntp_server: config.time_config.ntp_server,
                ntp_client_settings: NTPClientSettings {
                    timeout: config.time_config.timeout,
                    listening_interface: config.time_config.interface,
                },
                update_interval: config.time_config.update_interval,
                slot_config: SlotConfig {
                    slot_duration: config.time_config.slot_duration,
                    chain_start_time: config.time_config.chain_start_time,
                },
                epoch_config: config.consensus_config.ledger_config.epoch_config,
                base_period_length: config.consensus_config.ledger_config.base_period_length(),
            },
        },
        mempool: MempoolConfig {
            cl_pool_recovery_path: "./recovery/cl_mempool.json".into(),
            da_pool_recovery_path: "./recovery/da_mempool.json".into(),
        },
        membership: config.membership_config.service_settings,
        sdp: (),

        testing_http: nomos_api::ApiServiceSettings {
            backend_settings: AxumBackendSettings {
                address: testing_http_address,
                cors_origins: vec![],
            },
            request_timeout: None,
        },
    }
}
