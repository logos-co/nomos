use core::time::Duration;

use fixed::types::U57F7;
use nomos_da_network_core::{
    protocols::sampling::SubnetsConfig,
    swarm::{DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig},
};
use nomos_da_verifier::backend::trigger::MempoolPublishTriggerConfig;
use nomos_libp2p::protocol_name::StreamProtocol;
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};

use crate::config::deployment::Settings as DeploymentSettings;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    #[serde(flatten)]
    pub common: CommonSettings,
    pub network: NetworkSettings,
    pub verifier: VerifierSettings,
    pub sampling: SamplingSettings,
}

impl From<DeploymentSettings> for Settings {
    fn from(value: DeploymentSettings) -> Self {
        match value {
            DeploymentSettings::Mainnet => mainnet_settings(),
            DeploymentSettings::Custom(custom) => custom.da,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CommonSettings {
    // Protocol identifiers (used by network layer for all protocols)
    pub replication_protocol_name: StreamProtocol,
    pub dispersal_protocol_name: StreamProtocol,
    pub sampling_protocol_name: StreamProtocol,

    // Shared topology parameter
    pub num_subnets: usize,

    // Shared cryptographic parameters
    pub global_params_path: String,
    pub domain_size: usize,
}

/// Network-specific deployment settings.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkSettings {
    // Topology parameters for membership construction
    pub subnetwork_size: usize,
    pub replication_factor: usize,

    // Network service configuration
    pub subnet_refresh_interval: Duration,
    pub subnet_threshold: usize,
    pub min_session_members: usize,

    // Connection management
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
    pub replication_settings: ReplicationConfig,

    pub subnets_settings: SubnetsConfig,
}

/// Verifier-specific deployment settings.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VerifierSettings {
    pub mempool_trigger_settings: MempoolPublishTriggerConfig,
}

/// Sampling-specific deployment settings.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SamplingSettings {
    pub num_samples: u16,
    pub old_blobs_check_interval: Duration,
    pub blobs_validity_duration: Duration,
    pub commitments_wait_duration: Duration,
    pub sdp_blob_trigger_sampling_delay: Duration,
    pub shares_retry_limit: usize,
    pub commitments_retry_limit: usize,
}

/// Dispersal-specific deployment settings (executor only).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DispersalSettings {
    pub dispersal_timeout: Duration,
    pub retry_cooldown: Duration,
    pub retry_limit: usize,
}

#[must_use]
pub fn mainnet_settings() -> Settings {
    let common = CommonSettings {
        replication_protocol_name: StreamProtocol::new("/nomos/da/1.0.0/replication"),
        dispersal_protocol_name: StreamProtocol::new("/nomos/da/1.0.0/dispersal"),
        sampling_protocol_name: StreamProtocol::new("/nomos/da/1.0.0/sampling"),

        num_subnets: 16,
        global_params_path: "/etc/nomos/kzgrs_params".to_owned(),
        domain_size: 16,
    };

    let sampling = SamplingSettings {
        num_samples: 10,
        old_blobs_check_interval: Duration::from_secs(10),
        blobs_validity_duration: Duration::from_secs(600),
        commitments_wait_duration: Duration::from_secs(2),
        sdp_blob_trigger_sampling_delay: Duration::from_secs(5),
        shares_retry_limit: 5,
        commitments_retry_limit: 5,
    };

    let network = NetworkSettings {
        // Topology
        subnetwork_size: 32,
        replication_factor: 2,

        // Network service config
        subnet_refresh_interval: Duration::from_secs(30),
        subnet_threshold: 2048,
        min_session_members: 2,

        // Connection management
        policy_settings: DAConnectionPolicySettings {
            min_dispersal_peers: 8,
            min_replication_peers: 8,
            max_dispersal_failures: 3,
            max_sampling_failures: 3,
            max_replication_failures: 3,
            malicious_threshold: 5,
        },
        monitor_settings: DAConnectionMonitorSettings {
            failure_time_window: Duration::from_secs(10),
            time_decay_factor: U57F7::from_num(0.8),
        },
        balancer_interval: Duration::from_secs(5),
        redial_cooldown: Duration::from_secs(5),
        replication_settings: ReplicationConfig {
            seen_message_cache_size: 1000,
            seen_message_ttl: Duration::from_secs(3600),
        },

        // Construct SubnetsConfig from common and sampling params
        subnets_settings: SubnetsConfig {
            num_of_subnets: common.num_subnets,
            shares_retry_limit: sampling.shares_retry_limit,
            commitments_retry_limit: sampling.commitments_retry_limit,
        },
    };

    let verifier = VerifierSettings {
        mempool_trigger_settings: MempoolPublishTriggerConfig {
            publish_threshold: NonNegativeF64::try_from(0.8).unwrap(),
            share_duration: Duration::from_secs(5),
            prune_duration: Duration::from_secs(30),
            prune_interval: Duration::from_secs(5),
        },
    };

    // let dispersal = DispersalSettings {
    //     dispersal_timeout: Duration::from_secs(20),
    //     retry_cooldown: Duration::from_secs(5),
    //     retry_limit: 2,
    // };

    Settings {
        common,
        network,
        verifier,
        sampling,
    }
}
