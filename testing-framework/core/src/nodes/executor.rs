use std::{
    collections::HashSet,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use broadcast_service::BlockInfo;
use chain_service::CryptarchiaInfo;
use futures::Stream;
pub use integration_configs::nodes::executor::create_executor_config;
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::{
    block::Block, da::BlobId, header::HeaderId, mantle::SignedMantleTx, sdp::SessionNumber,
};
use nomos_da_network_core::swarm::{BalancerStats, MonitorStats};
use nomos_da_network_service::MembershipResponse;
use nomos_executor::config::Config;
use nomos_http_api_common::paths::{DA_GET_SHARES_COMMITMENTS, MANTLE_METRICS, MEMPOOL_ADD_TX};
use nomos_network::backends::libp2p::Libp2pInfo;
use nomos_node::api::testing::handlers::HistoricSamplingRequest;
use nomos_tracing::logging::local::FileConfig;
use nomos_tracing_service::LoggerLayer;
use reqwest::Url;
use tempfile::NamedTempFile;

use super::{ApiClient, create_tempdir, persist_tempdir, should_persist_tempdir};
use crate::{IS_DEBUG_TRACING, adjust_timeout, nodes::LOGS_PREFIX};

const BIN_PATH: &str = "target/debug/nomos-executor";

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(BIN_PATH)
}

pub struct Executor {
    tempdir: tempfile::TempDir,
    child: Child,
    config: Config,
    api: ApiClient,
}

impl Drop for Executor {
    fn drop(&mut self) {
        if should_persist_tempdir()
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
        .await
        .unwrap();

        node
    }

    pub async fn block_peer(&self, peer_id: String) -> bool {
        self.api.block_peer(&peer_id).await.unwrap()
    }

    pub async fn unblock_peer(&self, peer_id: String) -> bool {
        self.api.unblock_peer(&peer_id).await.unwrap()
    }

    pub async fn blacklisted_peers(&self) -> Vec<String> {
        self.api.blacklisted_peers().await.unwrap()
    }

    async fn wait_online(&self) {
        loop {
            let res = self.api.get_response(MANTLE_METRICS).await;
            if res.is_ok() && res.unwrap().status().is_success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    #[must_use]
    pub fn url(&self) -> Url {
        self.api.base_url().clone()
    }

    #[must_use]
    pub fn testing_url(&self) -> Option<Url> {
        self.api.testing_url()
    }

    pub async fn balancer_stats(&self) -> BalancerStats {
        self.api.balancer_stats().await.unwrap()
    }

    pub async fn monitor_stats(&self) -> MonitorStats {
        self.api.monitor_stats().await.unwrap()
    }

    pub async fn network_info(&self) -> Libp2pInfo {
        self.api.network_info().await.unwrap()
    }

    pub async fn consensus_info(&self) -> CryptarchiaInfo {
        self.api.consensus_info().await.unwrap()
    }

    pub async fn get_block(&self, id: HeaderId) -> Option<Block<SignedMantleTx>> {
        self.api.storage_block(&id).await.unwrap()
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

    pub async fn get_commitments(&self, blob_id: BlobId) -> Option<DaSharesCommitments> {
        self.api
            .post_json_decode(DA_GET_SHARES_COMMITMENTS, &blob_id)
            .await
            .unwrap()
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

    pub async fn da_get_membership(
        &self,
        session_id: SessionNumber,
    ) -> Result<MembershipResponse, reqwest::Error> {
        self.api.da_get_membership(&session_id).await
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

    pub async fn get_lib_stream(
        &self,
    ) -> Result<impl Stream<Item = BlockInfo>, common_http_client::Error> {
        self.api
            .http_client()
            .get_lib_stream(self.api.base_url().clone())
            .await
    }

    pub async fn add_tx(&self, tx: SignedMantleTx) -> Result<(), reqwest::Error> {
        self.api.post_json_unit(MEMPOOL_ADD_TX, &tx).await
    }
}
