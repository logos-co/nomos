use core::{num::NonZeroU64, time::Duration};

use nomos_blend_service::{
    core::settings::{CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings},
    settings::TimingSettings,
};
use nomos_da_network_core::{
    protocols::sampling::SubnetsConfig,
    swarm::{DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig},
};
use nomos_da_verifier::backend::trigger::MempoolPublishTriggerConfig;
use nomos_libp2p::protocol_name::StreamProtocol;
use nomos_node::config::{
    blend::deployment::{
        CommonSettings as BlendCommonSettings, CoreSettings as BlendCoreSettings,
        Settings as BlendDeploymentSettings,
    },
    da::deployment::{
        CommonSettings as DaCommonSettings, DispersalSettings as DaDispersalSettings,
        NetworkSettings as DaNetworkSettings, SamplingSettings as DaSamplingSettings,
        Settings as DaDeploymentSettings, VerifierSettings as DaVerifierSettings,
    },
    deployment::{CustomDeployment, Settings as DeploymentSettings},
    network::deployment::Settings as NetworkDeploymentSettings,
};
use nomos_utils::math::NonNegativeF64;

use crate::{adjust_timeout, topology::configs::da::GLOBAL_PARAMS_PATH};

#[expect(clippy::too_many_lines, reason = "Big config constructor")]
#[must_use]
pub fn default_e2e_deployment_settings() -> DeploymentSettings {
    DeploymentSettings::Custom(Box::new(CustomDeployment {
        blend: BlendDeploymentSettings {
            common: BlendCommonSettings {
                minimum_network_size: NonZeroU64::try_from(30u64)
                    .expect("Minimum network size cannot be zero."),
                num_blend_layers: NonZeroU64::try_from(3)
                    .expect("Number of blend layers cannot be zero."),
                timing: TimingSettings {
                    round_duration: Duration::from_secs(1),
                    rounds_per_interval: NonZeroU64::try_from(30u64)
                        .expect("Rounds per interval cannot be zero."),
                    // (21,600 blocks * 30s per block) / 1s per round = 648,000 rounds
                    rounds_per_session: NonZeroU64::try_from(648_000u64)
                        .expect("Rounds per session cannot be zero."),
                    rounds_per_observation_window: NonZeroU64::try_from(30u64)
                        .expect("Rounds per observation window cannot be zero."),
                    rounds_per_session_transition_period: NonZeroU64::try_from(30u64)
                        .expect("Rounds per session transition period cannot be zero."),
                    epoch_transition_period_in_slots: NonZeroU64::try_from(2_600)
                        .expect("Epoch transition period in slots cannot be zero."),
                },
                protocol_name: StreamProtocol::new("/blend/integration-tests"),
            },
            core: BlendCoreSettings {
                minimum_messages_coefficient: NonZeroU64::try_from(1)
                    .expect("Minimum messages coefficient cannot be zero."),
                normalization_constant: 1.03f64
                    .try_into()
                    .expect("Normalization constant cannot be negative."),
                scheduler: SchedulerSettings {
                    cover: CoverTrafficSettings {
                        intervals_for_safety_buffer: 100,
                        message_frequency_per_round: NonNegativeF64::try_from(1f64)
                            .expect("Message frequency per round cannot be negative."),
                    },
                    delayer: MessageDelayerSettings {
                        maximum_release_delay_in_rounds: NonZeroU64::try_from(3u64)
                            .expect("Maximum release delay between rounds cannot be zero."),
                    },
                },
            },
        },
        network: NetworkDeploymentSettings {
            identify_protocol_name: StreamProtocol::new("/integration/nomos/identify/1.0.0"),
            kademlia_protocol_name: StreamProtocol::new("/integration/nomos/kad/1.0.0"),
        },
        da: DaDeploymentSettings {
            common: DaCommonSettings {
                replication_protocol_name: StreamProtocol::new("/nomos/da/replication/1.0.0"),
                dispersal_protocol_name: StreamProtocol::new("/nomos/da/dispersal/1.0.0"),
                sampling_protocol_name: StreamProtocol::new("/nomos/da/sampling/1.0.0"),
                num_subnets: 2,
                global_params_path: GLOBAL_PARAMS_PATH.clone(),
                domain_size: 2,
            },
            network: DaNetworkSettings {
                subnetwork_size: 2,
                replication_factor: 1,
                subnet_refresh_interval: Duration::from_secs(30),
                subnet_threshold: 2,
                min_session_members: 2,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: 1,
                    min_replication_peers: 1,
                    max_dispersal_failures: 0,
                    max_sampling_failures: 0,
                    max_replication_failures: 0,
                    malicious_threshold: 0,
                },
                monitor_settings: DAConnectionMonitorSettings {
                    failure_time_window: Duration::from_secs(5),
                    ..Default::default()
                },
                balancer_interval: Duration::from_secs(1),
                redial_cooldown: Duration::ZERO,
                replication_settings: ReplicationConfig {
                    seen_message_cache_size: 1000,
                    seen_message_ttl: Duration::from_secs(3600),
                },
                subnets_settings: SubnetsConfig {
                    num_of_subnets: 1,
                    shares_retry_limit: 1,
                    commitments_retry_limit: 1,
                },
            },
            verifier: DaVerifierSettings {
                mempool_trigger_settings: MempoolPublishTriggerConfig {
                    publish_threshold: NonNegativeF64::try_from(0.8).unwrap(),
                    share_duration: Duration::from_secs(5),
                    prune_duration: Duration::from_secs(30),
                    prune_interval: Duration::from_secs(5),
                },
            },
            sampling: DaSamplingSettings {
                num_samples: 1,
                old_blobs_check_interval: Duration::from_secs(5),
                blobs_validity_duration: Duration::from_secs(60),
                commitments_wait_duration: Duration::from_secs(1),
                sdp_blob_trigger_sampling_delay: adjust_timeout(Duration::from_secs(5)),
                shares_retry_limit: 1,
                commitments_retry_limit: 1,
            },
            dispersal: DaDispersalSettings {
                dispersal_timeout: Duration::from_secs(20),
                retry_cooldown: Duration::from_secs(3),
                retry_limit: 2,
            },
        },
    }))
}

#[must_use]
pub fn get_e2e_custom_settings() -> CustomDeployment {
    match default_e2e_deployment_settings() {
        DeploymentSettings::Custom(custom) => *custom,
        DeploymentSettings::Mainnet => unreachable!("tests use custom deployment"),
    }
}
