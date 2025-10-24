use std::num::NonZeroU64;

use nomos_blend_scheduling::message_blend::crypto::SessionCryptographicProcessorSettings;
use nomos_core::mantle::keys::{PublicKey, SecretKey};
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};

use crate::settings::TimingSettings;

#[derive(Clone, Debug)]
pub struct BlendConfig<BackendSettings> {
    pub backend: BackendSettings,
    pub crypto: SessionCryptographicProcessorSettings,
    pub scheduler: SchedulerSettings,
    pub time: TimingSettings,
    pub zk: ZkSettings,
    pub minimum_network_size: NonZeroU64,
}

impl<BackendSettings> BlendConfig<BackendSettings> {
    pub fn session_quota(&self, membership_size: usize) -> u64 {
        self.scheduler
            .cover
            .session_quota(&self.crypto, &self.time, membership_size)
    }

    pub(super) fn scheduler_settings(&self) -> nomos_blend_scheduling::message_scheduler::Settings {
        nomos_blend_scheduling::message_scheduler::Settings {
            additional_safety_intervals: self.scheduler.cover.intervals_for_safety_buffer,
            expected_intervals_per_session: self.time.intervals_per_session(),
            maximum_release_delay_in_rounds: self.scheduler.delayer.maximum_release_delay_in_rounds,
            round_duration: self.time.round_duration,
            rounds_per_interval: self.time.rounds_per_interval,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SchedulerSettings {
    #[serde(flatten)]
    pub cover: CoverTrafficSettings,
    #[serde(flatten)]
    pub delayer: MessageDelayerSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoverTrafficSettings {
    /// `F_c`: frequency at which cover messages are generated per round.
    pub message_frequency_per_round: NonNegativeF64,
    /// `R_c`: redundancy parameter for cover messages.
    pub redundancy_parameter: u64,
    // `max`: safety buffer length, expressed in intervals
    pub intervals_for_safety_buffer: u64,
}

#[cfg(test)]
impl Default for CoverTrafficSettings {
    fn default() -> Self {
        Self {
            intervals_for_safety_buffer: 1,
            message_frequency_per_round: 1.try_into().unwrap(),
            redundancy_parameter: 1,
        }
    }
}

impl CoverTrafficSettings {
    #[must_use]
    pub fn session_quota(
        &self,
        crypto: &SessionCryptographicProcessorSettings,
        timings: &TimingSettings,
        membership_size: usize,
    ) -> u64 {
        // `C`: Expected number of cover messages that are generated during a session by
        // the core nodes.
        let expected_number_of_session_messages =
            timings.rounds_per_session.get() as f64 * self.message_frequency_per_round.get();

        // `Q_c`: Messaging allowance that can be used by a core node during a
        // single session.
        let core_quota = ((expected_number_of_session_messages
            * (crypto.num_blend_layers + self.redundancy_parameter * crypto.num_blend_layers)
                as f64)
            / membership_size as f64)
            .ceil();

        // `c`: Maximal number of cover messages a node can generate per session.
        (core_quota / crypto.num_blend_layers as f64).ceil() as u64
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageDelayerSettings {
    pub maximum_release_delay_in_rounds: NonZeroU64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ZkSettings {
    #[serde(rename = "secret_key")]
    pub sk: SecretKey,
}

impl ZkSettings {
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        self.sk.to_public_key()
    }
}
