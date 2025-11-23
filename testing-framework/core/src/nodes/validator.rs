use std::{
    collections::HashSet,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use broadcast_service::BlockInfo;
use chain_service::CryptarchiaInfo;
use futures::Stream;
pub use integration_configs::nodes::validator::create_validator_config;
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::{block::Block, da::BlobId, mantle::SignedMantleTx, sdp::SessionNumber};
use nomos_da_network_core::swarm::{BalancerStats, MonitorStats};
use nomos_da_network_service::MembershipResponse;
use nomos_http_api_common::paths::{CRYPTARCHIA_HEADERS, DA_GET_SHARES_COMMITMENTS};
use nomos_network::backends::libp2p::Libp2pInfo;
use nomos_node::{Config, HeaderId, api::testing::handlers::HistoricSamplingRequest};
use nomos_tracing::logging::local::FileConfig;
use nomos_tracing_service::LoggerLayer;
use reqwest::Url;
use tempfile::NamedTempFile;
use tokio::time::error::Elapsed;
use tx_service::MempoolMetrics;

use super::{ApiClient, create_tempdir, persist_tempdir, should_persist_tempdir};
use crate::{IS_DEBUG_TRACING, adjust_timeout, nodes::LOGS_PREFIX};

const BIN_PATH: &str = "target/debug/nomos-node";

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(BIN_PATH)
}

pub enum Pool {
    Da,
    Mantle,
}

pub struct Validator {
    tempdir: tempfile::TempDir,
    child: Child,
    config: Config,
    api: ApiClient,
}

impl Drop for Validator {
    fn drop(&mut self) {
        if should_persist_tempdir()
            && let Err(e) = persist_tempdir(&mut self.tempdir, "nomos-node")
        {
            println!("failed to persist tempdir: {e}");
        }

        if let Err(e) = self.child.kill() {
            println!("failed to kill the child process: {e}");
        }
    }
}

impl Validator {
    /// Check if the validator process is still running
    pub fn is_running(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => false,
        }
    }

    /// Wait for the validator process to exit, with a timeout
    /// Returns true if the process exited within the timeout, false otherwise
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                if !self.is_running() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .is_ok()
    }

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

        let addr = config.http.backend_settings.address;
        let testing_addr = config.testing_http.backend_settings.address;

        serde_yaml::to_writer(&mut file, &config).unwrap();
        let child = Command::new(binary_path())
            .arg(&config_path)
            .current_dir(dir.path())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let node = Self {
            child,
            tempdir: dir,
            config,
            api: ApiClient::new(addr, Some(testing_addr)),
        };

        tokio::time::timeout(adjust_timeout(Duration::from_secs(10)), async {
            node.wait_online().await;
        })
        .await?;

        Ok(node)
    }

    #[must_use]
    pub fn url(&self) -> Url {
        self.api.base_url().clone()
    }

    #[must_use]
    pub fn testing_url(&self) -> Option<Url> {
        self.api.testing_url()
    }

    async fn wait_online(&self) {
        loop {
            if self.api.consensus_info().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn get_block(&self, id: HeaderId) -> Option<Block<SignedMantleTx>> {
        self.api.storage_block(&id).await.unwrap()
    }

    pub async fn get_commitments(&self, blob_id: BlobId) -> Option<DaSharesCommitments> {
        self.api
            .post_json_decode(DA_GET_SHARES_COMMITMENTS, &blob_id)
            .await
            .unwrap()
    }

    pub async fn get_mempoool_metrics(&self, pool: Pool) -> MempoolMetrics {
        let discr = match pool {
            Pool::Mantle => "mantle",
            Pool::Da => "da",
        };
        let res = self.api.mempool_metrics(discr).await.unwrap();
        MempoolMetrics {
            pending_items: res["pending_items"].as_u64().unwrap() as usize,
            last_item_timestamp: res["last_item_timestamp"].as_u64().unwrap(),
        }
    }

    pub async fn da_historic_sampling<I>(
        &self,
        block_id: HeaderId,
        blob_ids: I,
    ) -> Result<bool, reqwest::Error>
    where
        I: IntoIterator<Item = (BlobId, SessionNumber)>,
    {
        let request = HistoricSamplingRequest {
            block_id,
            blob_ids: blob_ids.into_iter().collect(),
        };

        self.api.da_historic_sampling(&request).await
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
        let mut req = self.api.get_builder(CRYPTARCHIA_HEADERS);

        if let Some(from) = from {
            req = req.query(&[("from", from)]);
        }

        if let Some(to) = to {
            req = req.query(&[("to", to)]);
        }

        let res = self.api.get_headers_raw(req).await;

        println!("res: {res:?}");

        res.unwrap().json::<Vec<HeaderId>>().await.unwrap()
    }

    pub async fn consensus_info(&self) -> CryptarchiaInfo {
        let info = self.api.consensus_info().await.unwrap();
        println!("{info:?}");
        info
    }

    pub async fn balancer_stats(&self) -> BalancerStats {
        self.api.balancer_stats().await.unwrap()
    }

    pub async fn monitor_stats(&self) -> MonitorStats {
        self.api.monitor_stats().await.unwrap()
    }

    pub async fn da_get_membership(
        &self,
        session_id: SessionNumber,
    ) -> Result<MembershipResponse, reqwest::Error> {
        self.api.da_get_membership(&session_id).await
    }

    pub async fn network_info(&self) -> Libp2pInfo {
        self.api.network_info().await.unwrap()
    }

    pub async fn get_shares(
        &self,
        blob_id: BlobId,
        requested_shares: HashSet<[u8; 2]>,
        filter_shares: HashSet<[u8; 2]>,
        return_available: bool,
    ) -> Result<impl Stream<Item = DaLightShare>, common_http_client::Error> {
        self.api
            .http_client()
            .get_shares::<DaShare>(
                self.api.base_url().clone(),
                blob_id,
                requested_shares,
                filter_shares,
                return_available,
            )
            .await
    }

    pub async fn get_storage_commitments(
        &self,
        blob_id: BlobId,
    ) -> Result<Option<DaSharesCommitments>, common_http_client::Error> {
        self.api
            .http_client()
            .get_storage_commitments::<DaShare>(self.api.base_url().clone(), blob_id)
            .await
    }

    pub async fn get_lib_stream(
        &self,
    ) -> Result<impl Stream<Item = BlockInfo>, common_http_client::Error> {
        self.api
            .http_client()
            .get_lib_stream(self.api.base_url().clone())
            .await
    }
}
