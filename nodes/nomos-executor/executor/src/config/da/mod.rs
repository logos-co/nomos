pub mod deployment;
pub mod dispersal;

use deployment::Settings as DeploymentSettings;
use dispersal::Config as DispersalUserConfig;
use nomos_da_dispersal::{
    DispersalServiceSettings,
    backend::kzgrs::{DispersalKZGRSBackendSettings, EncoderSettings},
};
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig,
    api::http::ApiAdapterSettings as DaNetworkApiAdapterSettings,
    backends::libp2p::{common::DaNetworkBackendSettings, executor::DaNetworkExecutorBackend},
};
use nomos_da_sampling::{
    DaSamplingServiceSettings, backend::kzgrs::KzgrsSamplingBackendSettings,
    verifier::kzgrs::KzgrsDaVerifierSettings as SamplingVerifierSettings,
};
use nomos_da_verifier::{DaVerifierServiceSettings, backend::kzgrs::KzgrsDaVerifierSettings};
use nomos_node::NomosDaMembership;
use serde::{Deserialize, Serialize};

use crate::RuntimeServiceId;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(flatten)]
    pub validator: nomos_node::config::da::Config,
    pub dispersal: DispersalUserConfig,
}

/// `ServiceConfig` combines user-provided configuration with
/// deployment-specific settings
#[derive(Clone)]
pub struct ServiceConfig {
    pub user: Config,
    pub deployment: DeploymentSettings,
}

type DaNetworkSettings = DaNetworkConfig<
    DaNetworkExecutorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaNetworkApiAdapterSettings,
    RuntimeServiceId,
>;

type DaVerifierSettings = DaVerifierServiceSettings<KzgrsDaVerifierSettings, ()>;

type DaSamplingSettings =
    DaSamplingServiceSettings<KzgrsSamplingBackendSettings, SamplingVerifierSettings>;

type DaDispersalSettings = DispersalServiceSettings<DispersalKZGRSBackendSettings>;

/// Converts `ServiceConfig` into a tuple of all DA service settings for
/// executor Returns (Network, Verifier, Sampling, Dispersal)
impl From<ServiceConfig>
    for (
        DaNetworkSettings,
        DaVerifierSettings,
        DaSamplingSettings,
        DaDispersalSettings,
    )
{
    fn from(config: ServiceConfig) -> Self {
        // Reuse validator conversion for verifier and sampling (they're identical)
        let validator_config = nomos_node::config::da::ServiceConfig {
            user: nomos_node::config::da::Config {
                network: config.user.validator.network.clone(),
            },
            deployment: nomos_node::config::da::deployment::Settings {
                common: config.deployment.validator.common.clone(),
                network: config.deployment.validator.network.clone(),
                verifier: config.deployment.validator.verifier.clone(),
                sampling: config.deployment.validator.sampling.clone(),
            },
        };
        let (_, verifier_settings, sampling_settings) = validator_config.into();

        // Executor-specific network settings (uses ExecutorBackend)
        let network_settings = DaNetworkSettings {
              backend: nomos_da_network_service::backends::libp2p::executor::DaNetworkExecutorBackendSettings {
                  validator_settings: DaNetworkBackendSettings {
                      // User values
                      node_key: config.user.validator.network.node_key,
                      listening_address: config.user.validator.network.listening_address,

                      // Deployment values
                      policy_settings: config.deployment.validator.network.policy_settings,
                      monitor_settings: config.deployment.validator.network.monitor_settings,
                      balancer_interval: config.deployment.validator.network.balancer_interval,
                      redial_cooldown: config.deployment.validator.network.redial_cooldown,
                      replication_settings: config.deployment.validator.network.replication_settings,
                      subnets_settings: config.deployment.validator.network.subnets_settings,
                  },
                  num_subnets: config.deployment.validator.common.num_subnets as u16,
              },

              membership: NomosDaMembership::new(
                  0,
                  config.deployment.validator.network.subnetwork_size,
                  config.deployment.validator.network.replication_factor,
              ),

              // User values
              api_adapter_settings: DaNetworkApiAdapterSettings {
                  api_port: config.user.validator.network.api_port,
                  is_secure: config.user.validator.network.is_secure,
              },

              // Deployment values
              subnet_refresh_interval: config.deployment.validator.network.subnet_refresh_interval,
              subnet_threshold: config.deployment.validator.network.subnet_threshold,
              min_session_members: config.deployment.validator.network.min_session_members,
          };

        // Dispersal settings (executor-only)
        let dispersal_settings = DaDispersalSettings {
            backend: DispersalKZGRSBackendSettings {
                encoder_settings: EncoderSettings {
                    // Deployment value
                    num_columns: config.deployment.validator.common.num_subnets,

                    // User values
                    with_cache: config.user.dispersal.encoder_settings.with_cache,
                    global_params_path: config.user.dispersal.encoder_settings.global_params_path,
                },

                // Deployment values
                dispersal_timeout: config.deployment.dispersal.dispersal_timeout,
                retry_cooldown: config.deployment.dispersal.retry_cooldown,
                retry_limit: config.deployment.dispersal.retry_limit,
            },
        };

        (
            network_settings,
            verifier_settings,
            sampling_settings,
            dispersal_settings,
        )
    }
}
