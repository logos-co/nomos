use core::{num::NonZeroU64, time::Duration};

use nomos_blend_service::{
    core::settings::{CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings},
    settings::TimingSettings,
};
use nomos_libp2p::protocol_name::StreamProtocol;
use nomos_node::config::{
    blend::deployment::{
        CommonSettings as BlendCommonSettings, CoreSettings as BlendCoreSettings,
        Settings as BlendDeploymentSettings,
    },
    deployment::{CustomDeployment, Settings as DeploymentSettings},
    network::deployment::Settings as NetworkDeploymentSettings,
};
use nomos_utils::math::NonNegativeF64;

#[must_use]
pub fn default_e2e_deployment_settings() -> DeploymentSettings {
    DeploymentSettings::Custom(CustomDeployment {
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
    })
}
