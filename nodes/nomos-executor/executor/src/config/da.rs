use nomos_da_dispersal::{DispersalServiceSettings, backend::kzgrs::DispersalKZGRSBackendSettings};
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig,
    api::http::ApiAdapterSettings as DaNetworkApiAdapterSettings,
    backends::libp2p::{
        common::DaNetworkBackendSettings,
        executor::{DaNetworkExecutorBackend, DaNetworkExecutorBackendSettings},
    },
};
use nomos_da_sampling::{
    DaSamplingServiceSettings, backend::kzgrs::KzgrsSamplingBackendSettings,
    verifier::kzgrs::KzgrsDaVerifierSettings as SamplingVerifierSettings,
};
use nomos_da_verifier::{DaVerifierServiceSettings, backend::kzgrs::KzgrsDaVerifierSettings};
use nomos_node::{NomosDaMembership, config::da::ServiceConfig as DaServiceConfig};

use crate::RuntimeServiceId;

// Type aliases for executor DA settings
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

/// Convert DA `ServiceConfig` to executor-specific settings (uses
/// `DaNetworkExecutorBackend`)
#[must_use]
pub fn da_config_to_executor_settings(
    config: DaServiceConfig,
) -> (
    DaNetworkSettings,
    DaVerifierSettings,
    DaSamplingSettings,
    Option<DaDispersalSettings>,
) {
    let (_, verifier_settings, sampling_settings, dispersal_settings) = config.clone().into();

    // Build executor-specific network settings (only difference is the backend
    // type)
    let network_settings = DaNetworkSettings {
        backend: DaNetworkExecutorBackendSettings {
            validator_settings: DaNetworkBackendSettings {
                node_key: config.user.network.node_key,
                listening_address: config.user.network.listening_address,
                policy_settings: config.deployment.network.policy_settings,
                monitor_settings: config.deployment.network.monitor_settings,
                balancer_interval: config.deployment.network.balancer_interval,
                redial_cooldown: config.deployment.network.redial_cooldown,
                replication_settings: config.deployment.network.replication_settings,
                subnets_settings: config.deployment.network.subnets_settings,
            },
            num_subnets: config.deployment.common.num_subnets as u16,
        },
        membership: NomosDaMembership::new(
            0,
            config.deployment.network.subnetwork_size,
            config.deployment.network.replication_factor,
        ),
        api_adapter_settings: DaNetworkApiAdapterSettings {
            api_port: config.user.network.api_port,
            is_secure: config.user.network.is_secure,
        },
        subnet_refresh_interval: config.deployment.network.subnet_refresh_interval,
        subnet_threshold: config.deployment.network.subnet_threshold,
        min_session_members: config.deployment.network.min_session_members,
    };

    (
        network_settings,
        verifier_settings,
        sampling_settings,
        dispersal_settings,
    )
}
