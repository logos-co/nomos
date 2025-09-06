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
    /// Builds a stream of [`SessionInfo`] by wrapping a stream of
    /// [`SessionEvent`].
    ///
    /// For each [`SessionEvent::NewSession`] event received, the stream
    /// constructs a new [`SessionInfo`] based on the new membership.
    /// [`SessionEvent::TransitionPeriodExpired`] events are ignored.
    pub(super) fn session_info_stream(
        &self,
        initial_session: &Membership<NodeId>,
        session_event_stream: impl Stream<Item = SessionEvent<Membership<NodeId>>> + Send + 'static,
    ) -> (SessionInfo, impl Stream<Item = SessionInfo> + Unpin)
    where
        BackendSettings: Clone + Send + 'static,
        NodeId: Clone + Send + 'static,
    {
        let settings = self.clone();
        let initial_session_info = SessionInfo {
            core_quota: settings.scheduler.cover.session_quota(
                &settings.crypto,
                &settings.time,
                initial_session.size(),
            ),
            session_number: 0u128.into(),
        };

        let session_info_stream = session_event_stream
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
                        SessionEvent::TransitionPeriodExpired => None, // Ignore this event
                    }
                }
            })
            .enumerate()
            .map(|(session_number, core_quota)| SessionInfo {
                core_quota,
                // Add 1 to the session number since the initial session number is 0.
                session_number: (session_number
                    .checked_add(1)
                    .expect("session number must not overflow")
                    as u128)
                    .into(),
            })
            .boxed();

        (initial_session_info, session_info_stream)
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

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;
    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;
    use crate::test_utils::membership::membership;

    #[tokio::test]
    async fn session_info_stream() {
        let settings = settings(1.0, 10);

        let (session_sender, session_receiver) = mpsc::channel(1);
        let (initial_session_info, mut stream) = settings.session_info_stream(
            &membership(&[NodeId(0)], NodeId(0)),
            ReceiverStream::new(session_receiver),
        );

        // Expect that the initial session info has the session number 0.
        assert_eq!(initial_session_info.session_number, 0u128.into());

        // Feed a transition period expired event (should be ignored).
        session_sender
            .send(SessionEvent::TransitionPeriodExpired)
            .await
            .unwrap();
        // No new session info since the event was ignored.
        assert!(stream.next().now_or_never().is_none());

        // Feed a new session event with a bigger membership.
        session_sender
            .send(SessionEvent::NewSession(membership(
                &[NodeId(0), NodeId(1)],
                NodeId(0),
            )))
            .await
            .unwrap();
        // Expect a new session info with session number 1.
        let info = stream
            .next()
            .now_or_never()
            .expect("should yield immediately")
            .expect("shouldn't be closed");
        assert_eq!(info.session_number, 1u128.into());
        // Expect the core quota to be different due to the different membership size.
        assert_ne!(info.core_quota, initial_session_info.core_quota);

        // The stream should yield `None` if the underlying stream is closed.
        drop(session_sender);
        assert!(stream
            .next()
            .now_or_never()
            .expect("should yield immediately")
            .is_none());
    }

    fn settings(
        message_frequency_per_round: f64,
        rounds_per_session: u64,
    ) -> BlendConfig<(), NodeId> {
        BlendConfig {
            backend: (),
            crypto: CryptographicProcessorSettings {
                signing_private_key: Ed25519PrivateKey::generate(),
                num_blend_layers: 1,
            },
            scheduler: SchedulerSettingsExt {
                cover: CoverTrafficSettingsExt {
                    message_frequency_per_round: NonNegativeF64::try_from(
                        message_frequency_per_round,
                    )
                    .unwrap(),
                    redundancy_parameter: 0,
                    intervals_for_safety_buffer: 0,
                },
                delayer: MessageDelayerSettingsExt {
                    maximum_release_delay_in_rounds: NonZeroU64::new(1).unwrap(),
                },
            },
            time: TimingSettings {
                rounds_per_session: NonZeroU64::new(rounds_per_session).unwrap(),
                rounds_per_interval: NonZeroU64::new(1).unwrap(),
                round_duration: std::time::Duration::from_secs(1),
                rounds_per_observation_window: NonZeroU64::new(1).unwrap(),
                rounds_per_session_transition_period: NonZeroU64::new(1).unwrap(),
            },
            membership: vec![],
            minimum_network_size: NonZeroU64::new(1).unwrap(),
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    struct NodeId(u8);

    impl From<NodeId> for [u8; 32] {
        fn from(id: NodeId) -> Self {
            [id.0; 32]
        }
    }
}
