use std::{fs, net::Ipv4Addr, num::NonZero, path::PathBuf, sync::Arc, time::Duration};

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use nomos_da_dispersal::backend::kzgrs::MempoolPublishStrategy;
use nomos_da_network_core::swarm::{
    DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
};
use nomos_tracing_service::TracingSettings;
use serde::{Deserialize, Serialize};
use tests::{
    nodes::{executor::create_executor_config, validator::create_validator_config},
    topology::configs::{consensus::ConsensusParams, da::DaParams},
};
use tokio::sync::oneshot::channel;

use crate::{
    config::Host,
    repo::{ConfigRepo, RepoResponse},
};

#[derive(Debug, Deserialize)]
pub struct CfgSyncConfig {
    pub port: u16,
    pub n_hosts: usize,
    pub timeout: u64,

    // ConsensusConfig related parameters
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,

    // DaConfig related parameters
    pub subnetwork_size: usize,
    pub dispersal_factor: usize,
    pub num_samples: u16,
    pub num_subnets: u16,
    pub old_blobs_check_interval_secs: u64,
    pub blobs_validity_duration_secs: u64,
    pub global_params_path: String,
    pub min_dispersal_peers: usize,
    pub min_replication_peers: usize,
    pub monitor_failure_time_window: Duration,
    pub balancer_interval_secs: u64,
    pub mempool_publish_strategy: MempoolPublishStrategy,
    pub replication_settings: ReplicationConfig,

    // Tracing params
    pub tracing_settings: TracingSettings,
}

impl CfgSyncConfig {
    pub fn load_from_file(file_path: &PathBuf) -> Result<Self, String> {
        let config_content = fs::read_to_string(file_path)
            .map_err(|err| format!("Failed to read config file: {err}"))?;
        serde_yaml::from_str(&config_content)
            .map_err(|err| format!("Failed to parse config file: {err}"))
    }

    #[must_use]
    pub const fn to_consensus_params(&self) -> ConsensusParams {
        ConsensusParams {
            n_participants: self.n_hosts,
            security_param: self.security_param,
            active_slot_coeff: self.active_slot_coeff,
        }
    }

    #[must_use]
    pub fn to_da_params(&self) -> DaParams {
        DaParams {
            subnetwork_size: self.subnetwork_size,
            dispersal_factor: self.dispersal_factor,
            num_samples: self.num_samples,
            num_subnets: self.num_subnets,
            old_blobs_check_interval: Duration::from_secs(self.old_blobs_check_interval_secs),
            blobs_validity_duration: Duration::from_secs(self.blobs_validity_duration_secs),
            global_params_path: self.global_params_path.clone(),
            mempool_strategy: self.mempool_publish_strategy.clone(),
            policy_settings: DAConnectionPolicySettings {
                min_dispersal_peers: self.min_dispersal_peers,
                min_replication_peers: self.min_replication_peers,
                max_dispersal_failures: 3,
                max_sampling_failures: 3,
                max_replication_failures: 3,
                malicious_threshold: 10,
            },
            monitor_settings: DAConnectionMonitorSettings {
                failure_time_window: self.monitor_failure_time_window,
                ..Default::default()
            },
            balancer_interval: Duration::from_secs(self.balancer_interval_secs),
            redial_cooldown: Duration::ZERO,
            replication_settings: self.replication_settings,
        }
    }

    #[must_use]
    pub fn to_tracing_settings(&self) -> TracingSettings {
        self.tracing_settings.clone()
    }
}

#[derive(Serialize, Deserialize)]
pub struct ClientIp {
    pub ip: Ipv4Addr,
    pub identifier: String,
}

async fn validator_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp { ip, identifier } = payload;

    let (reply_tx, reply_rx) = channel();
    config_repo.register(Host::default_validator_from_ip(ip, identifier), reply_tx);

    (reply_rx.await).map_or_else(
        |_| (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response(),
        |config_response| match config_response {
            RepoResponse::Config(config) => {
                let config = create_validator_config(*config);
                (StatusCode::OK, Json(config)).into_response()
            }
            RepoResponse::Timeout => (StatusCode::REQUEST_TIMEOUT).into_response(),
        },
    )
}

async fn executor_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp { ip, identifier } = payload;

    let (reply_tx, reply_rx) = channel();
    config_repo.register(Host::default_executor_from_ip(ip, identifier), reply_tx);

    (reply_rx.await).map_or_else(
        |_| (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response(),
        |config_response| match config_response {
            RepoResponse::Config(config) => {
                let config = create_executor_config(*config);
                (StatusCode::OK, Json(config)).into_response()
            }
            RepoResponse::Timeout => (StatusCode::REQUEST_TIMEOUT).into_response(),
        },
    )
}

pub fn cfgsync_app(config_repo: Arc<ConfigRepo>) -> Router {
    Router::new()
        .route("/validator", post(validator_config))
        .route("/executor", post(executor_config))
        .with_state(config_repo)
}
