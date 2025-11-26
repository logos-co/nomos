use nomos_da_dispersal::{
    DispersalServiceSettings,
    backend::kzgrs::{DispersalKZGRSBackendSettings, EncoderSettings},
};
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

    let verifier_settings = DaVerifierSettings {
        share_verifier_settings: KzgrsDaVerifierSettings {
            global_params_path: config.deployment.common.global_params_path.clone(),
            domain_size: config.deployment.common.domain_size,
        },
        tx_verifier_settings: (),

        mempool_trigger_settings: config.deployment.verifier.mempool_trigger_settings,
    };

    let sampling_settings = DaSamplingSettings {
        sampling_settings: KzgrsSamplingBackendSettings {
            num_samples: config.deployment.sampling.num_samples,
            num_subnets: config.deployment.common.num_subnets as u16,
            old_blobs_check_interval: config.deployment.sampling.old_blobs_check_interval,
            blobs_validity_duration: config.deployment.sampling.blobs_validity_duration,
        },
        share_verifier_settings: SamplingVerifierSettings {
            global_params_path: config.deployment.common.global_params_path,
            domain_size: config.deployment.common.domain_size,
        },
        commitments_wait_duration: config.deployment.sampling.commitments_wait_duration,
        sdp_blob_trigger_sampling_delay: config.deployment.sampling.sdp_blob_trigger_sampling_delay,
    };

    let dispersal_settings = config
        .user
        .dispersal
        .map(|dispersal_config| DaDispersalSettings {
            backend: DispersalKZGRSBackendSettings {
                encoder_settings: EncoderSettings {
                    num_columns: config.deployment.common.num_subnets,
                    with_cache: dispersal_config.encoder_settings.with_cache,
                    global_params_path: dispersal_config.encoder_settings.global_params_path,
                },
                dispersal_timeout: config.deployment.dispersal.dispersal_timeout,
                retry_cooldown: config.deployment.dispersal.retry_cooldown,
                retry_limit: config.deployment.dispersal.retry_limit,
            },
        });

    (
        network_settings,
        verifier_settings,
        sampling_settings,
        dispersal_settings,
    )
}
