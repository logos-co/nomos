use std::{fs, net::Ipv4Addr, num::NonZero, path::PathBuf, sync::Arc, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use integration_configs::{
    nodes::{create_executor_config, create_validator_config},
    topology::configs::{consensus::ConsensusParams, da::DaParams},
};
use nomos_da_network_core::swarm::{
    DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
};
use nomos_tracing_service::TracingSettings;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::serde_as;
use tokio::sync::oneshot::channel;

use crate::{
    config::{
        DEFAULT_API_PORT, DEFAULT_BLEND_PORT, DEFAULT_DA_NETWORK_PORT, DEFAULT_LIBP2P_NETWORK_PORT,
        DEFAULT_TESTING_HTTP_PORT, Host,
    },
    repo::{ConfigRepo, RepoResponse},
};

#[serde_as]
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
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub old_blobs_check_interval: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub blobs_validity_duration: Duration,
    pub global_params_path: String,
    pub min_dispersal_peers: usize,
    pub min_replication_peers: usize,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub monitor_failure_time_window: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub balancer_interval: Duration,
    pub replication_settings: ReplicationConfig,
    pub retry_shares_limit: usize,
    pub retry_commitments_limit: usize,

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
            old_blobs_check_interval: self.old_blobs_check_interval,
            blobs_validity_duration: self.blobs_validity_duration,
            global_params_path: self.global_params_path.clone(),
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
            balancer_interval: self.balancer_interval,
            redial_cooldown: Duration::ZERO,
            replication_settings: self.replication_settings,
            subnets_refresh_interval: Duration::from_secs(30),
            retry_shares_limit: self.retry_shares_limit,
            retry_commitments_limit: self.retry_commitments_limit,
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
    #[serde(default)]
    pub network_port: Option<u16>,
    #[serde(default)]
    pub da_network_port: Option<u16>,
    #[serde(default)]
    pub blend_port: Option<u16>,
    #[serde(default)]
    pub api_port: Option<u16>,
    #[serde(default)]
    pub testing_http_port: Option<u16>,
}

async fn validator_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp {
        ip,
        identifier,
        network_port,
        da_network_port,
        blend_port,
        api_port,
        testing_http_port,
    } = payload;

    println!(
        "validator request: ip={ip} identifier={identifier} network_port={network_port:?} da_port={da_network_port:?} blend_port={blend_port:?} api_port={api_port:?} testing_http_port={testing_http_port:?}"
    );

    let (reply_tx, reply_rx) = channel();
    let host = Host::validator(
        ip,
        identifier,
        network_port.unwrap_or(DEFAULT_LIBP2P_NETWORK_PORT),
        da_network_port.unwrap_or(DEFAULT_DA_NETWORK_PORT),
        blend_port.unwrap_or(DEFAULT_BLEND_PORT),
        api_port.unwrap_or(DEFAULT_API_PORT),
        testing_http_port.unwrap_or(DEFAULT_TESTING_HTTP_PORT),
    );
    config_repo.register(host, reply_tx);

    match reply_rx.await {
        Ok(RepoResponse::Config(config)) => {
            println!("validator config ready");
            let config = create_validator_config(*config);
            match sanitize_config(config) {
                Ok(value) => (StatusCode::OK, Json(value)).into_response(),
                Err(err) => {
                    println!("validator config serialization failed: {err}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Error serializing config",
                    )
                        .into_response()
                }
            }
        }
        Ok(RepoResponse::Timeout) => {
            println!("validator config timeout");
            (StatusCode::REQUEST_TIMEOUT).into_response()
        }
        Err(_) => {
            println!("validator config channel closed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response()
        }
    }
}

async fn executor_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp {
        ip,
        identifier,
        network_port,
        da_network_port,
        blend_port,
        api_port,
        testing_http_port,
    } = payload;

    println!(
        "executor request: ip={ip} identifier={identifier} network_port={network_port:?} da_port={da_network_port:?} blend_port={blend_port:?} api_port={api_port:?} testing_http_port={testing_http_port:?}"
    );

    let (reply_tx, reply_rx) = channel();
    let host = Host::executor(
        ip,
        identifier,
        network_port.unwrap_or(DEFAULT_LIBP2P_NETWORK_PORT),
        da_network_port.unwrap_or(DEFAULT_DA_NETWORK_PORT),
        blend_port.unwrap_or(DEFAULT_BLEND_PORT),
        api_port.unwrap_or(DEFAULT_API_PORT.saturating_add(1)),
        testing_http_port.unwrap_or(DEFAULT_TESTING_HTTP_PORT),
    );
    config_repo.register(host, reply_tx);

    match reply_rx.await {
        Ok(RepoResponse::Config(config)) => {
            println!("executor config ready");
            let config = create_executor_config(*config);
            match sanitize_config(config) {
                Ok(value) => (StatusCode::OK, Json(value)).into_response(),
                Err(err) => {
                    println!("executor config serialization failed: {err}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Error serializing config",
                    )
                        .into_response()
                }
            }
        }
        Ok(RepoResponse::Timeout) => {
            println!("executor config timeout");
            (StatusCode::REQUEST_TIMEOUT).into_response()
        }
        Err(_) => {
            println!("executor config channel closed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response()
        }
    }
}

pub fn cfgsync_app(config_repo: Arc<ConfigRepo>) -> Router {
    Router::new()
        .route("/validator", post(validator_config))
        .route("/executor", post(executor_config))
        .with_state(config_repo)
}

fn sanitize_config<T>(config: T) -> Result<Value, serde_json::Error>
where
    T: Serialize,
{
    serde_json::to_value(config)
}
