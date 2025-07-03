use std::{num::NonZeroU64, time::Duration};

use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
    message_scheduler::session_info::SessionInfo,
};
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub scheduler: SchedulerSettingsExt,
    pub time: TimingSettings,
    pub membership: Vec<Node<NodeId>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SchedulerSettingsExt {
    #[serde(flatten)]
    pub cover: CoverTrafficSettingsExt,
    #[serde(flatten)]
    pub delayer: MessageDelayerSettingsExt,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoverTrafficSettingsExt {
    /// `F_c`: frequency at which cover messages are generated per round.
    pub message_frequency_per_round: NonNegativeF64,
    /// `R_c`: redundancy parameter for cover messages.
    pub redundancy_parameter: u64,
    // `max`: safety buffer length, expressed in intervals
    pub intervals_for_safety_buffer: u64,
}

impl CoverTrafficSettingsExt {
    fn session_quota(
        &self,
        crypto: &CryptographicProcessorSettings,
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
pub struct MessageDelayerSettingsExt {
    pub maximum_release_delay_in_rounds: NonZeroU64,
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TimingSettings {
    /// `S`: length of a session in terms of expected rounds (on average).
    pub rounds_per_session: NonZeroU64,
    /// `|I|`: length of an interval in terms of rounds.
    pub rounds_per_interval: NonZeroU64,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    /// Duration of a round.
    pub round_duration: Duration,
    pub rounds_per_observation_window: NonZeroU64,
}

impl TimingSettings {
    #[must_use]
    pub const fn session_duration(&self) -> Duration {
        Duration::from_secs(self.rounds_per_session.get() * self.round_duration.as_secs())
    }

    #[must_use]
    pub fn intervals_per_session(&self) -> NonZeroU64 {
        NonZeroU64::try_from(self.rounds_per_session.get() / self.rounds_per_interval.get()).expect("Obtained `0` when calculating the number of intervals per session, which is not allowed.")
    }
}

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId>
where
    NodeId: Clone,
{
    pub(super) fn membership(&self) -> Membership<NodeId> {
        let local_signing_pubkey = self.crypto.signing_private_key.public_key();
        Membership::new(self.membership.clone(), &local_signing_pubkey)
    }
}

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId> {
    pub(super) fn session_stream(&self) -> impl Stream<Item = SessionInfo> {
        let membership_size = self.membership.len() + 1;
        let static_quota_for_membership =
            self.scheduler
                .cover
                .session_quota(&self.crypto, &self.time, membership_size);
        IntervalStream::new(interval(self.time.session_duration()))
            .enumerate()
            .map(move |(session_number, _)| SessionInfo {
                core_quota: static_quota_for_membership,
                session_number: (session_number as u128).into(),
            })
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
