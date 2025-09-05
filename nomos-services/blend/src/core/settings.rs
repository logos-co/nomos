use std::num::NonZeroU64;

use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
    message_scheduler::session_info::SessionInfo,
    session::SessionEvent,
};
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};

use crate::settings::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub scheduler: SchedulerSettingsExt,
    pub time: TimingSettings,
    // TODO: Remove this and use the membership service stream instead: https://github.com/logos-co/nomos/issues/1532
    pub membership: Vec<Node<NodeId>>,
    pub minimum_network_size: NonZeroU64,
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

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId> {
    pub(super) fn session_info_stream(
        &self,
        session_event_stream: impl Stream<Item = SessionEvent<Membership<NodeId>>> + Send + 'static,
    ) -> impl Stream<Item = SessionInfo> + Unpin
    where
        BackendSettings: Clone + Send + 'static,
        NodeId: Clone + Send + 'static,
    {
        let settings = self.clone();
        session_event_stream
            .filter_map(move |event| {
                let settings = settings.clone();
                async move {
                    match event {
                        SessionEvent::NewSession(membership) => {
                            Some(settings.scheduler.cover.session_quota(
                                &settings.crypto,
                                &settings.time,
                                membership.size(),
                            ))
                        }
                        SessionEvent::TransitionPeriodExpired => None,
                    }
                }
            })
            .enumerate()
            .map(|(session_number, core_quota)| SessionInfo {
                core_quota,
                session_number: (session_number as u128).into(),
            })
            .boxed()
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
