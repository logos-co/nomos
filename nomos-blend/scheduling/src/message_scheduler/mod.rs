use core::{
    fmt::Debug,
    marker::PhantomData,
    num::NonZeroU64,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    stream::{empty, AbortHandle},
    Stream, StreamExt as _,
};
use tracing::{info, trace};

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{
        round_info::{RoundClock, RoundInfo},
        session_info::SessionInfo,
        utils::setup_new_session,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

pub mod round_info;
pub mod session_info;
mod utils;

#[cfg(test)]
mod tests;

const LOG_TARGET: &str = "blend::scheduling";

/// A message scheduler waiting for the session stream to provide consumable
/// session info, to compute session-related components to pass to its
/// constituent sub-streams.
pub struct UninitializedMessageScheduler<SessionClock, Rng, ProcessedMessage> {
    /// The random generator to select rounds for message releasing.
    rng: Rng,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
    _phantom: PhantomData<ProcessedMessage>,
}

impl<SessionClock, Rng, ProcessedMessage>
    UninitializedMessageScheduler<SessionClock, Rng, ProcessedMessage>
{
    pub const fn new(session_clock: SessionClock, settings: Settings, rng: Rng) -> Self {
        Self {
            rng,
            session_clock,
            settings,
            _phantom: PhantomData,
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage>
    UninitializedMessageScheduler<SessionClock, Rng, ProcessedMessage>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone,
{
    /// Waits until the provided [`SessionClock`] returns the first tick with
    /// the relevant session information.
    pub async fn wait_next_session_start(
        self,
    ) -> MessageScheduler<SessionClock, Rng, ProcessedMessage> {
        let Self {
            rng,
            mut session_clock,
            settings,
            ..
        } = self;
        // We wait until the provided session stream returns its first usable value,
        // which we use to initialize the scheduler.
        let first_session_info = async {
            loop {
                if let Some(session_info) = session_clock.next().await {
                    break session_info;
                }
            }
        }
        .await;

        MessageScheduler::new(session_clock, first_session_info, rng, settings)
    }
}

/// The initialized version of [`UninitializedMessageScheduler`] that is created
/// after the session stream yields its first result.
pub struct MessageScheduler<SessionClock, Rng, ProcessedMessage> {
    /// The module responsible for randomly generated cover messages, given the
    /// allowed session quota and accounting for data messages generated within
    /// the session.
    cover_traffic: SessionCoverTraffic<RoundClock>,
    /// The module responsible for delaying the release of processed messages
    /// that have not been fully decapsulated.
    release_delayer: SessionProcessedMessageDelayer<RoundClock, Rng, ProcessedMessage>,
    /// The clock ticking at the beginning of each new round.
    round_clock: RoundClock,
    /// The abort handle to kill the round stream broadcaster at the beginning
    /// of each new session.
    round_clock_task_abort_handle: AbortHandle,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
}

impl<SessionClock, Rng, ProcessedMessage> MessageScheduler<SessionClock, Rng, ProcessedMessage>
where
    Rng: rand::Rng + Clone,
{
    fn new(
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
        let mut initial_release_delayer =
            SessionProcessedMessageDelayer::<_, _, ProcessedMessage>::new(
                crate::release_delayer::Settings {
                    maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
                },
                rng.clone(),
                Box::new(empty()) as RoundClock,
            );
        let mut initial_round_clock = Box::new(empty()) as RoundClock;
        let (mut initial_round_clock_task_abort_handle, _) = AbortHandle::new_pair();

        setup_new_session(
            &mut initial_cover_traffic,
            &mut initial_release_delayer,
            &mut initial_round_clock,
            &mut initial_round_clock_task_abort_handle,
            settings,
            rng,
            initial_session_info,
        );

        Self {
            cover_traffic: initial_cover_traffic,
            release_delayer: initial_release_delayer,
            round_clock: initial_round_clock,
            round_clock_task_abort_handle: initial_round_clock_task_abort_handle,
            session_clock,
            settings,
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage> MessageScheduler<SessionClock, Rng, ProcessedMessage> {
    /// Notify the cover message submodule that a new data message has been
    /// generated in this session, which will reduce the number of cover
    /// messages generated going forward.
    pub fn notify_new_data_message(&mut self) {
        self.cover_traffic.notify_new_data_message();
    }

    /// Add a new processed message to the release delayer component queue, for
    /// release during the next release window.
    pub fn schedule_message(&mut self, message: ProcessedMessage) {
        self.release_delayer.schedule_message(message);
    }

    #[cfg(test)]
    pub fn with_test_values(
        cover_traffic: SessionCoverTraffic<RoundClock>,
        release_delayer: SessionProcessedMessageDelayer<RoundClock, Rng, ProcessedMessage>,
        round_clock: RoundClock,
        session_clock: SessionClock,
    ) -> Self {
        Self {
            cover_traffic,
            release_delayer,
            round_clock,
            round_clock_task_abort_handle: AbortHandle::new_pair().0,
            session_clock,
            // These are not needed when all fields are provided as arguments.
            settings: Settings::default(),
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage> Drop
    for MessageScheduler<SessionClock, Rng, ProcessedMessage>
{
    fn drop(&mut self) {
        trace!(target: LOG_TARGET, "Dropping message scheduler.");
        self.round_clock_task_abort_handle.abort();
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
            round_clock_task_abort_handle,
            cover_traffic,
            release_delayer,
            round_clock,
            settings,
            session_clock,
        } = &mut *self;
        // We update session info on new sessions.
        let rng = release_delayer.rng().clone();
        match session_clock.poll_next_unpin(cx) {
            Poll::Ready(Some(new_session_info)) => {
                setup_new_session(
                    cover_traffic,
                    release_delayer,
                    round_clock,
                    round_clock_task_abort_handle,
                    *settings,
                    rng,
                    new_session_info,
                );
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        // We do not return anything if a new round has not elapsed.
        let new_round = match round_clock.poll_next_unpin(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Ready(Some(new_round)) => new_round,
        };
        trace!(target: LOG_TARGET, "New round {new_round} started.");

        // We poll the sub-stream and return the right result accordingly.
        let cover_traffic_output = cover_traffic.poll_next_unpin(cx);
        let release_delayer_output = release_delayer.poll_next_unpin(cx);

        match (cover_traffic_output, release_delayer_output) {
            // If none of the sub-streams is ready, we do not return anything.
            (Poll::Pending, Poll::Pending) => {
                // Awake to trigger a new round clock tick.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            // Bubble up `Poll::Ready(None)` if any sub-stream returns it.
            (Poll::Ready(None), _) | (_, Poll::Ready(None)) => Poll::Ready(None),
            // If at least one sub-stream yields a result, we yield a new result, which might also
            // contain no actual elements if the cover message module did not yield a new cover
            // message and there were no queue messages to be released in this release
            // window.
            (cover_message_ready_output, processed_messages_ready_output) => {
                let cover_message =
                    (cover_message_ready_output == Poll::Ready(Some(()))).then_some(());
                let processed_messages = if let Poll::Ready(Some(processed_messages)) =
                    processed_messages_ready_output
                {
                    processed_messages
                } else {
                    vec![]
                };
                let round_info = RoundInfo {
                    processed_messages,
                    cover_message_generation_flag: cover_message,
                };
                info!(target: LOG_TARGET, "Emitting new round info {round_info:?}.");
                Poll::Ready(Some(round_info))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub additional_safety_intervals: u64,
    pub expected_intervals_per_session: NonZeroU64,
    pub maximum_release_delay_in_rounds: NonZeroU64,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroU64,
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
        }
    }
}
