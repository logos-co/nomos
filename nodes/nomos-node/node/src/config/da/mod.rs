use nomos_da_dispersal::{
    DispersalServiceSettings,
    backend::kzgrs::{DispersalKZGRSBackendSettings, EncoderSettings},
};
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig,
    api::http::ApiAdapterSettings as DaNetworkApiAdapterSettings,
    backends::libp2p::{common::DaNetworkBackendSettings, validator::DaNetworkValidatorBackend},
};
use nomos_da_sampling::{
    DaSamplingServiceSettings, backend::kzgrs::KzgrsSamplingBackendSettings,
    verifier::kzgrs::KzgrsDaVerifierSettings as SamplingVerifierSettings,
};
use nomos_da_verifier::{DaVerifierServiceSettings, backend::kzgrs::KzgrsDaVerifierSettings};
use serde::{Deserialize, Serialize};

use crate::{
    NomosDaMembership, RuntimeServiceId,
    config::da::{
        deployment::Settings as DeploymentSettings, dispersal::Config as DispersalUserConfig,
        network::Config as NetworkUserConfig, sampling::Config as SamplingUserConfig,
        verifier::Config as VerifierUserConfig,
    },
};

pub mod deployment;
pub mod dispersal;
pub mod network;
pub mod sampling;
pub mod verifier;

/// Parent DA configuration containing all DA services (network, verifier,
/// sampling, and optionally dispersal for executors)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub network: NetworkUserConfig,
    pub verifier: VerifierUserConfig,
    pub sampling: SamplingUserConfig,
    /// Dispersal config - only present for executor nodes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispersal: Option<DispersalUserConfig>,
}

/// `ServiceConfig` combines user-provided configuration with
/// deployment-specific settings
pub struct ServiceConfig {
    pub user: Config,
    pub deployment: DeploymentSettings,
}

type DaNetworkSettings = DaNetworkConfig<
    DaNetworkValidatorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaNetworkApiAdapterSettings,
    RuntimeServiceId,
>;

type DaVerifierSettings = DaVerifierServiceSettings<KzgrsDaVerifierSettings, ()>;

type DaSamplingSettings =
    DaSamplingServiceSettings<KzgrsSamplingBackendSettings, SamplingVerifierSettings>;

type DaDispersalSettings = DispersalServiceSettings<DispersalKZGRSBackendSettings>;

/// Converts `ServiceConfig` into a tuple of all DA service settings
/// Returns (Network, Verifier, Sampling, Optional Dispersal)
impl From<ServiceConfig>
    for (
        DaNetworkSettings,
        DaVerifierSettings,
        DaSamplingSettings,
        Option<DaDispersalSettings>,
    )
{
    fn from(config: ServiceConfig) -> Self {
        // Network settings
        let network_settings = DaNetworkSettings {
            backend: DaNetworkBackendSettings {
                // User values
                node_key: config.user.network.node_key,
                listening_address: config.user.network.listening_address,

                // Deployment values
                policy_settings: config.deployment.network.policy_settings,
                monitor_settings: config.deployment.network.monitor_settings,
                balancer_interval: config.deployment.network.balancer_interval,
                redial_cooldown: config.deployment.network.redial_cooldown,
                replication_settings: config.deployment.network.replication_settings,
                subnets_settings: config.deployment.network.subnets_settings,
            },

            membership: NomosDaMembership::new(
                0,
                config.deployment.network.subnetwork_size,
                config.deployment.network.replication_factor,
            ),

            // User values
            api_adapter_settings: DaNetworkApiAdapterSettings {
                api_port: config.user.network.api_port,
                is_secure: config.user.network.is_secure,
            },

            // Deployment values
            subnet_refresh_interval: config.deployment.network.subnet_refresh_interval,
            subnet_threshold: config.deployment.network.subnet_threshold,
            min_session_members: config.deployment.network.min_session_members,
        };

        let verifier_settings = DaVerifierSettings {
            share_verifier_settings: KzgrsDaVerifierSettings {
                // Deployment values
                global_params_path: config.deployment.common.global_params_path.clone(),
                domain_size: config.deployment.common.domain_size,
            },

            tx_verifier_settings: (),

            // Deployment values
            mempool_trigger_settings: config.deployment.verifier.mempool_trigger_settings,
        };

        let sampling_settings = DaSamplingSettings {
            sampling_settings: KzgrsSamplingBackendSettings {
                // All deployment values
                num_samples: config.deployment.sampling.num_samples,
                num_subnets: config.deployment.common.num_subnets as u16,
                old_blobs_check_interval: config.deployment.sampling.old_blobs_check_interval,
                blobs_validity_duration: config.deployment.sampling.blobs_validity_duration,
            },

            share_verifier_settings: SamplingVerifierSettings {
                // Deployment values
                global_params_path: config.deployment.common.global_params_path,
                domain_size: config.deployment.common.domain_size,
            },

            // Deployment values
            commitments_wait_duration: config.deployment.sampling.commitments_wait_duration,
            sdp_blob_trigger_sampling_delay: config
                .deployment
                .sampling
                .sdp_blob_trigger_sampling_delay,
        };

        // Dispersal settings (optional - only for executors)
        let dispersal_settings = config.user.dispersal.map(|dispersal_config| {
            DaDispersalSettings {
                backend: DispersalKZGRSBackendSettings {
                    encoder_settings: EncoderSettings {
                        // deployment
                        num_columns: config.deployment.common.num_subnets,
                        // User values
                        with_cache: dispersal_config.encoder_settings.with_cache,
                        global_params_path: dispersal_config.encoder_settings.global_params_path,
                    },
                    // Deployment values
                    dispersal_timeout: config.deployment.dispersal.dispersal_timeout,
                    retry_cooldown: config.deployment.dispersal.retry_cooldown,
                    retry_limit: config.deployment.dispersal.retry_limit,
                },
            }
        });

        (
            network_settings,
            verifier_settings,
            sampling_settings,
            dispersal_settings,
        )
    }
}
