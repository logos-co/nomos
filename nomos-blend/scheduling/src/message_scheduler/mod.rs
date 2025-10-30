use core::{
    fmt::Debug,
    num::NonZeroU64,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Stream, StreamExt as _, stream::empty};
use tracing::{debug, info, trace};

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{
        message_queue::MessageQueue,
        round_info::{RoundClock, RoundInfo},
        session_info::SessionInfo,
        utils::setup_new_session,
    },
    release_clock::SessionReleaseClock,
};

mod message_queue;
pub mod round_info;
pub mod session_info;
mod utils;

#[cfg(test)]
mod tests;

const LOG_TARGET: &str = "blend::scheduling";

/// The initialized version of [`UninitializedMessageScheduler`] that is created
/// after the session stream yields its first result.
pub struct MessageScheduler<SessionClock, Rng, ProcessedMessage> {
    /// The module responsible for randomly generated cover messages, given the
    /// allowed session quota and accounting for data messages generated within
    /// the session.
    cover_traffic: SessionCoverTraffic<RoundClock>,
    /// The release clock.
    release_clock: SessionReleaseClock<RoundClock, Rng>,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
    message_queue: MessageQueue<Rng, ProcessedMessage>,
    unprocessed_data_messages: usize,
}

impl<SessionClock, Rng, ProcessedMessage> MessageScheduler<SessionClock, Rng, ProcessedMessage>
where
    Rng: rand::Rng + Clone,
{
    pub fn new(
        session_clock: SessionClock,
        initial_session_info: SessionInfo,
        mut rng: Rng,
        settings: Settings,
    ) -> Self {
        // To avoid duplication for the `setup_new_session` logic, we need to create
        // "dummy" containers that are soon replaced in the `setup_new_session`
        // function, which expects a `&mut` reference to those fields. The same function
        // is then called upon each new session tick.
        let mut initial_cover_traffic = SessionCoverTraffic::<RoundClock>::new(
            crate::cover_traffic::Settings {
                additional_safety_intervals: settings.additional_safety_intervals,
                expected_intervals_per_session: settings.expected_intervals_per_session,
                rounds_per_interval: settings.rounds_per_interval,
                starting_quota: initial_session_info.core_quota,
            },
            &mut rng,
            Box::new(empty()) as RoundClock,
        );
        let mut initial_release_clock = SessionReleaseClock::new(
            crate::release_clock::Settings {
                maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
            },
            rng.clone(),
            Box::new(empty()) as RoundClock,
        );

        setup_new_session(
            &mut initial_cover_traffic,
            &mut initial_release_clock,
            settings,
            rng.clone(),
            initial_session_info,
        );

        Self {
            cover_traffic: initial_cover_traffic,
            release_clock: initial_release_clock,
            session_clock,
            settings,
            message_queue: MessageQueue::new(rng),
            unprocessed_data_messages: 0,
        }
    }

    #[cfg(feature = "unsafe-test-functions")]
    pub const fn cover_traffic_module(&self) -> &SessionCoverTraffic<RoundClock> {
        &self.cover_traffic
    }
}

impl<SessionClock, Rng, ProcessedMessage> MessageScheduler<SessionClock, Rng, ProcessedMessage> {
    /// Notify the scheduler that a new data message has been
    /// generated in this session, which will reduce the number of cover
    /// messages generated going forward.
    pub fn notify_new_data_message(&mut self) {
        self.unprocessed_data_messages = self
            .unprocessed_data_messages
            .checked_add(1)
            .expect("Overflow when incrementing unprocessed data message count.");
        debug!(target: LOG_TARGET, "New data message event registered. Unprocessed messages count: {}", self.unprocessed_data_messages);
    }

    /// Add a new processed message to the release queue, for
    /// release during one of the future release windows.
    pub fn schedule_message(&mut self, message: ProcessedMessage) {
        self.message_queue.add_processed_message(message);
    }

    #[cfg(test)]
    pub fn with_test_values(
        cover_traffic: SessionCoverTraffic<RoundClock>,
        release_delayer: SessionReleaseClock<RoundClock, Rng>,
        session_clock: SessionClock,
        message_queue: MessageQueue<Rng, ProcessedMessage>,
        unprocessed_data_messages: usize,
    ) -> Self {
        Self {
            cover_traffic,
            release_clock: release_delayer,
            session_clock,
            // These are not needed when all fields are provided as arguments.
            settings: Settings::default(),
            message_queue,
            unprocessed_data_messages,
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage> Stream
    for MessageScheduler<SessionClock, Rng, ProcessedMessage>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone + Unpin,
    ProcessedMessage: Debug + Unpin,
{
    type Item = RoundInfo<ProcessedMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            cover_traffic,
            release_clock,
            settings,
            session_clock,
            message_queue,
            unprocessed_data_messages,
        } = &mut *self;
        // We update session info on new sessions.
        let rng = release_clock.rng().clone();
        match session_clock.poll_next_unpin(cx) {
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
            Poll::Ready(Some(new_session_info)) => {
                setup_new_session(
                    cover_traffic,
                    release_clock,
                    *settings,
                    rng,
                    new_session_info,
                );
            }
        }

        // Process any new generated cover message
        match cover_traffic.poll_next_unpin(cx) {
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
            Poll::Ready(Some(())) => {
                message_queue.add_cover_message();
            }
        }

        // We finally check if we are in a release round
        let release_round = match release_clock.poll_next_unpin(cx) {
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {
                return Poll::Pending;
            }
            Poll::Ready(Some(release_round)) => release_round,
        };

        let Some(release_message_batch) = message_queue.take_next_random_n_elements(
            settings.maximum_messages_released_per_round.get() as usize,
        ) else {
            trace!(target: LOG_TARGET, "No messages to release at this release round {release_round}.");
            // Awake to trigger polling of the next release round.
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let (processed_messages, cover_messages) = release_message_batch.into_components();

        let cover_messages_to_ignore = (*unprocessed_data_messages).min(cover_messages);
        *unprocessed_data_messages =
            unprocessed_data_messages.saturating_sub(cover_messages_to_ignore);

        let remaining_cover_messages = cover_messages.saturating_sub(cover_messages_to_ignore);

        let release_round_info = RoundInfo::new(processed_messages, remaining_cover_messages);
        info!(target: LOG_TARGET, "Emitting new round info {release_round_info:?}.");
        Poll::Ready(Some(release_round_info))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub additional_safety_intervals: u64,
    pub expected_intervals_per_session: NonZeroU64,
    pub maximum_release_delay_in_rounds: NonZeroU64,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroU64,
    pub maximum_messages_released_per_round: NonZeroU64,
}

#[cfg(test)]
impl Default for Settings {
    fn default() -> Self {
        Self {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroU64::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroU64::try_from(1).unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_interval: NonZeroU64::try_from(1).unwrap(),
            maximum_messages_released_per_round: NonZeroU64::try_from(1).unwrap(),
        }
    }
}
