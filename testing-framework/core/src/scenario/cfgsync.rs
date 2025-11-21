use std::{fs::File, num::NonZero, path::Path, time::Duration};

use anyhow::{Context as _, Result};
use nomos_da_network_core::swarm::ReplicationConfig;
use nomos_tracing::metrics::otlp::OtlpMetricsConfig;
use nomos_tracing_service::{MetricsLayer, TracingSettings};
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::topology::GeneratedTopology;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncConfig {
    pub port: u16,
    pub n_hosts: usize,
    pub timeout: u64,
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
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
    pub tracing_settings: TracingSettings,
}

pub fn load_cfgsync_template(path: &Path) -> Result<CfgSyncConfig> {
    let file = File::open(path)
        .with_context(|| format!("opening cfgsync template at {}", path.display()))?;
    serde_yaml::from_reader(file).context("parsing cfgsync template")
}

pub fn write_cfgsync_template(path: &Path, cfg: &CfgSyncConfig) -> Result<()> {
    let file = File::create(path)
        .with_context(|| format!("writing cfgsync template to {}", path.display()))?;
    let serializable = SerializableCfgSyncConfig::from(cfg);
    serde_yaml::to_writer(file, &serializable).context("serializing cfgsync template")
}

pub fn render_cfgsync_yaml(cfg: &CfgSyncConfig) -> Result<String> {
    let serializable = SerializableCfgSyncConfig::from(cfg);
    serde_yaml::to_string(&serializable).context("rendering cfgsync yaml")
}

pub fn apply_topology_overrides(
    cfg: &mut CfgSyncConfig,
    topology: &GeneratedTopology,
    use_kzg_mount: bool,
) {
    let hosts = topology.validators().len() + topology.executors().len();
    cfg.n_hosts = hosts;

    let consensus = &topology.config().consensus_params;
    cfg.security_param = consensus.security_param;
    cfg.active_slot_coeff = consensus.active_slot_coeff;

    let da = &topology.config().da_params;
    cfg.subnetwork_size = da.subnetwork_size;
    cfg.dispersal_factor = da.dispersal_factor;
    cfg.num_samples = da.num_samples;
    cfg.num_subnets = da.num_subnets;
    cfg.old_blobs_check_interval = da.old_blobs_check_interval;
    cfg.blobs_validity_duration = da.blobs_validity_duration;
    cfg.global_params_path = if use_kzg_mount {
        "/kzgrs_test_params".into()
    } else {
        da.global_params_path.clone()
    };
    cfg.min_dispersal_peers = da.policy_settings.min_dispersal_peers;
    cfg.min_replication_peers = da.policy_settings.min_replication_peers;
    cfg.monitor_failure_time_window = da.monitor_settings.failure_time_window;
    cfg.balancer_interval = da.balancer_interval;
    cfg.replication_settings = da.replication_settings;
    cfg.retry_shares_limit = da.retry_shares_limit;
    cfg.retry_commitments_limit = da.retry_commitments_limit;
    cfg.tracing_settings.metrics = MetricsLayer::Otlp(OtlpMetricsConfig {
        endpoint: Url::parse("http://prometheus:9090/api/v1/otlp/v1/metrics")
            .expect("valid prometheus otlp endpoint"),
        host_identifier: String::new(),
    });
}

#[serde_as]
#[derive(Serialize)]
struct SerializableCfgSyncConfig {
    port: u16,
    n_hosts: usize,
    timeout: u64,
    security_param: NonZero<u32>,
    active_slot_coeff: f64,
    subnetwork_size: usize,
    dispersal_factor: usize,
    num_samples: u16,
    num_subnets: u16,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    old_blobs_check_interval: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    blobs_validity_duration: Duration,
    global_params_path: String,
    min_dispersal_peers: usize,
    min_replication_peers: usize,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    monitor_failure_time_window: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    balancer_interval: Duration,
    replication_settings: ReplicationConfig,
    retry_shares_limit: usize,
    retry_commitments_limit: usize,
    tracing_settings: TracingSettings,
}

impl From<&CfgSyncConfig> for SerializableCfgSyncConfig {
    fn from(cfg: &CfgSyncConfig) -> Self {
        Self {
            port: cfg.port,
            n_hosts: cfg.n_hosts,
            timeout: cfg.timeout,
            security_param: cfg.security_param,
            active_slot_coeff: cfg.active_slot_coeff,
            subnetwork_size: cfg.subnetwork_size,
            dispersal_factor: cfg.dispersal_factor,
            num_samples: cfg.num_samples,
            num_subnets: cfg.num_subnets,
            old_blobs_check_interval: cfg.old_blobs_check_interval,
            blobs_validity_duration: cfg.blobs_validity_duration,
            global_params_path: cfg.global_params_path.clone(),
            min_dispersal_peers: cfg.min_dispersal_peers,
            min_replication_peers: cfg.min_replication_peers,
            monitor_failure_time_window: cfg.monitor_failure_time_window,
            balancer_interval: cfg.balancer_interval,
            replication_settings: cfg.replication_settings,
            retry_shares_limit: cfg.retry_shares_limit,
            retry_commitments_limit: cfg.retry_commitments_limit,
            tracing_settings: cfg.tracing_settings.clone(),
        }
    }
}
