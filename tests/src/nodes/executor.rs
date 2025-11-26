use std::{
    collections::HashSet,
    net::SocketAddr,
    num::NonZeroUsize,
    path::PathBuf,
    process::{Child, Command, Stdio},
    str::FromStr as _,
    time::Duration,
};

use broadcast_service::BlockInfo;
use chain_leader::LeaderSettings;
use chain_network::{ChainNetworkSettings, OrphanConfig, SyncConfig};
use chain_service::{CryptarchiaInfo, CryptarchiaSettings, StartingState};
use common_http_client::CommonHttpClient;
use cryptarchia_engine::time::SlotConfig;
use futures::Stream;
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::{
    block::Block, da::BlobId, header::HeaderId, mantle::SignedMantleTx, sdp::SessionNumber,
};
use nomos_da_network_core::swarm::{BalancerStats, MonitorStats};
use nomos_da_network_service::MembershipResponse;
use nomos_executor::{api::backend::AxumBackendSettings, config::Config};
use nomos_http_api_common::paths::{
    CRYPTARCHIA_INFO, DA_BALANCER_STATS, DA_BLACKLISTED_PEERS, DA_BLOCK_PEER, DA_GET_MEMBERSHIP,
    DA_GET_SHARES_COMMITMENTS, DA_HISTORIC_SAMPLING, DA_MONITOR_STATS, DA_UNBLOCK_PEER,
    MANTLE_METRICS, NETWORK_INFO, STORAGE_BLOCK,
};
use nomos_network::backends::libp2p::Libp2pInfo;
use nomos_node::{
    RocksBackendSettings,
    api::{handlers::GetCommitmentsRequest, testing::handlers::HistoricSamplingRequest},
    config::mempool::MempoolConfig,
};
use nomos_sdp::SdpSettings;
use nomos_time::{
    TimeServiceSettings,
    backends::{NtpTimeBackendSettings, ntp::async_client::NTPClientSettings},
};
use nomos_tracing::logging::local::FileConfig;
use nomos_tracing_service::LoggerLayer;
use nomos_utils::net::get_available_tcp_port;
use reqwest::Url;
use tempfile::NamedTempFile;

use super::{CLIENT, create_tempdir, persist_tempdir};
use crate::{
    IS_DEBUG_TRACING, adjust_timeout,
    nodes::{DA_GET_TESTING_ENDPOINT_ERROR, LOGS_PREFIX},
    topology::configs::{GeneralConfig, deployment::default_e2e_deployment_settings},
};

const BIN_PATH: &str = "../target/debug/nomos-executor";

pub struct Executor {
    addr: SocketAddr,
    testing_http_addr: SocketAddr,
    tempdir: tempfile::TempDir,
    child: Child,
    config: Config,
    http_client: CommonHttpClient,
}

impl Drop for Executor {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Err(e) = persist_tempdir(&mut self.tempdir, "nomos-executor")
        {
            println!("failed to persist tempdir: {e}");
        }

        if let Err(e) = self.child.kill() {
            println!("failed to kill the child process: {e}");
        }
    }
}

impl Executor {
    pub async fn spawn(mut config: Config) -> Self {
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
            http_client: CommonHttpClient::new_with_client(CLIENT.clone(), None),
        };
        tokio::time::timeout(adjust_timeout(Duration::from_secs(10)), async {
            node.wait_online().await;
        })
        .await
        .unwrap();

        node
    }

    pub async fn block_peer(&self, peer_id: String) -> bool {
        CLIENT
            .post(format!("http://{}{}", self.addr, DA_BLOCK_PEER))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&peer_id).unwrap())
            .send()
            .await
            .unwrap()
            .json::<bool>()
            .await
            .unwrap()
    }

    pub async fn unblock_peer(&self, peer_id: String) -> bool {
        CLIENT
            .post(format!("http://{}{}", self.addr, DA_UNBLOCK_PEER))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&peer_id).unwrap())
            .send()
            .await
            .unwrap()
            .json::<bool>()
            .await
            .unwrap()
    }

    pub async fn blacklisted_peers(&self) -> Vec<String> {
        CLIENT
            .get(format!("http://{}{}", self.addr, DA_BLACKLISTED_PEERS))
            .send()
            .await
            .unwrap()
            .json::<Vec<String>>()
            .await
            .unwrap()
    }

    async fn wait_online(&self) {
        loop {
            let res = self.get(MANTLE_METRICS).await;
            if res.is_ok() && res.unwrap().status().is_success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        CLIENT
            .get(format!("http://{}{}", self.addr, path))
            .send()
            .await
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
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

    pub async fn network_info(&self) -> Libp2pInfo {
        self.get(NETWORK_INFO).await.unwrap().json().await.unwrap()
    }

    pub async fn consensus_info(&self) -> CryptarchiaInfo {
        self.get(CRYPTARCHIA_INFO)
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    pub async fn get_block(&self, id: HeaderId) -> Option<Block<SignedMantleTx>> {
        CLIENT
            .post(format!("http://{}{}", self.addr, STORAGE_BLOCK))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&id).unwrap())
            .send()
            .await
            .unwrap()
            .json::<Option<Block<SignedMantleTx>>>()
            .await
            .unwrap()
    }

    pub async fn get_shares(
        &self,
        blob_id: BlobId,
        requested_shares: HashSet<[u8; 2]>,
        filter_shares: HashSet<[u8; 2]>,
        return_available: bool,
    ) -> Result<impl Stream<Item = DaLightShare>, common_http_client::Error> {
        self.http_client
            .get_shares::<DaShare>(
                Url::from_str(&format!("http://{}", self.addr))?,
                blob_id,
                requested_shares,
                filter_shares,
                return_available,
            )
            .await
    }

    pub async fn get_commitments(
        &self,
        blob_id: BlobId,
        session: SessionNumber,
    ) -> Option<DaSharesCommitments> {
        let request = GetCommitmentsRequest { blob_id, session };

        CLIENT
            .post(format!("http://{}{}", self.addr, DA_GET_SHARES_COMMITMENTS))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&request).unwrap())
            .send()
            .await
            .unwrap()
            .json::<Option<DaSharesCommitments>>()
            .await
            .unwrap()
    }

    pub async fn get_storage_commitments(
        &self,
        blob_id: BlobId,
    ) -> Result<Option<DaSharesCommitments>, common_http_client::Error> {
        self.http_client
            .get_storage_commitments::<DaShare>(
                Url::from_str(&format!("http://{}", self.addr))?,
                blob_id,
            )
            .await
    }

    pub async fn da_get_membership(
        &self,
        session_id: SessionNumber,
    ) -> Result<MembershipResponse, reqwest::Error> {
        let response = CLIENT
            .post(format!(
                "http://{}{}",
                self.testing_http_addr, DA_GET_MEMBERSHIP
            ))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&session_id).unwrap())
            .send()
            .await;

        assert!(response.is_ok(), "{}", DA_GET_TESTING_ENDPOINT_ERROR);

        let response = response.unwrap();
        response.error_for_status()?.json().await
    }

    pub async fn da_historic_sampling(
        &self,
        block_id: HeaderId,
        blob_ids: Vec<(BlobId, SessionNumber)>,
    ) -> Result<bool, reqwest::Error> {
        let request = HistoricSamplingRequest { block_id, blob_ids };

        let response = CLIENT
            .post(format!(
                "http://{}{}",
                self.testing_http_addr, DA_HISTORIC_SAMPLING
            ))
            .json(&request)
            .send()
            .await?;

        response.error_for_status_ref()?;

        // Parse the boolean response
        let success: bool = response.json().await?;
        Ok(success)
    }

    pub async fn get_lib_stream(
        &self,
    ) -> Result<impl Stream<Item = BlockInfo>, common_http_client::Error> {
        self.http_client
            .get_lib_stream(Url::from_str(&format!("http://{}", self.addr))?)
            .await
    }

    pub async fn add_tx(&self, tx: SignedMantleTx) -> Result<(), reqwest::Error> {
        CLIENT
            .post(format!(
                "http://{}{}",
                self.addr,
                nomos_http_api_common::paths::MEMPOOL_ADD_TX
            ))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&tx).unwrap())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[must_use]
#[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
pub fn create_executor_config(config: GeneralConfig) -> Config {
    let testing_http_address = format!("127.0.0.1:{}", get_available_tcp_port().unwrap())
        .parse()
        .unwrap();

    Config {
        network: config.network_config,
        blend: config.blend_config.0,
        deployment: default_e2e_deployment_settings(),
        cryptarchia: CryptarchiaSettings {
            config: config.consensus_config.ledger_config.clone(),
            starting_state: StartingState::Genesis {
                genesis_tx: config.consensus_config.genesis_tx,
            },
            recovery_file: PathBuf::from("./recovery/cryptarchia.json"),
            bootstrap: chain_service::BootstrapConfig {
                prolonged_bootstrap_period: Duration::from_secs(3),
                force_bootstrap: false,
                offline_grace_period: chain_service::OfflineGracePeriodConfig {
                    grace_period: Duration::from_secs(20 * 60),
                    state_recording_interval: Duration::from_secs(60),
                },
            },
        },
        chain_network: ChainNetworkSettings {
            config: config.consensus_config.ledger_config.clone(),
            network_adapter_settings:
                chain_network::network::adapters::libp2p::LibP2pAdapterSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
            bootstrap: chain_network::BootstrapConfig {
                ibd: chain_network::IbdConfig {
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
        cryptarchia_leader: LeaderSettings {
            transaction_selector_settings: (),
            config: config.consensus_config.ledger_config.clone(),
            leader_config: config.consensus_config.leader_config.clone(),
            blend_broadcast_settings:
                nomos_blend_service::core::network::libp2p::Libp2pBroadcastSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
        },

        tracing: config.tracing_config.tracing_settings,
        http: nomos_api::ApiServiceSettings {
            backend_settings: AxumBackendSettings {
                address: config.api_config.address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        da: nomos_node::config::da::Config {
            network: nomos_node::config::da::network::Config {
                node_key: config.da_config.node_key,
                listening_address: config.da_config.listening_address,
                api_port: config.api_config.address.port(),
                is_secure: false,
            },
            verifier: nomos_node::config::da::verifier::Config,
            sampling: nomos_node::config::da::sampling::Config,
            dispersal: Some(nomos_node::config::da::dispersal::Config {
                encoder_settings: nomos_node::config::da::dispersal::EncoderConfig {
                    with_cache: false,
                    global_params_path: config.da_config.global_params_path,
                },
            }),
        },
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
            pool_recovery_path: "./recovery/mempool.json".into(),
        },
        sdp: SdpSettings { declaration: None },
        wallet: nomos_wallet::WalletServiceSettings {
            known_keys: HashSet::from_iter([config.consensus_config.leader_config.pk]),
        },
        key_management: config.kms_config,

        testing_http: nomos_api::ApiServiceSettings {
            backend_settings: AxumBackendSettings {
                address: testing_http_address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
    }
}
