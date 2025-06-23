use core::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    stream::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use tokio::{
    sync::broadcast,
    time::{self, interval},
};
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tracing::trace;

use crate::{
    cover_traffic_2::SessionCoverTraffic,
    message::OutboundMessage,
    message_scheduler::{
        round_info::{Round, RoundInfo},
        session_info::SessionInfo,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

pub mod round_info;
pub mod session_info;

const LOG_TARGET: &str = "blend::scheduling";

/// A message scheduler waiting for the session stream to provide consumable
/// session info, to compute session-related components to pass to its
/// constituent sub-streams.
pub struct UninitializedMessageScheduler<SessionClock, Rng> {
    /// The random generator to select rounds for message releasing.
    rng: Rng,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng> {
    pub const fn new(session_clock: SessionClock, settings: Settings, rng: Rng) -> Self {
        Self {
            rng,
            session_clock,
            settings,
        }
    }
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng,
{
    /// Waits until the provided [`SessionClock`] returns the first tick with
    /// the relevant session information.
    pub async fn wait_next_session_start(self) -> MessageScheduler<SessionClock, Rng> {
        let Self {
            mut rng,
            mut session_clock,
            settings,
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

        MessageScheduler::new(session_clock, settings)
    }
}

/// The initialized version of [`UninitializedMessageScheduler`] that is created
/// after the session stream yields its first result.
pub struct MessageScheduler<SessionClock, Rng> {
    /// The module responsible for randomly generated cover messages, given the
    /// allowed session quota and accounting for data messages generated within
    /// the session.
    cover_traffic: SessionCoverTraffic<Box<dyn Stream<Item = Round>>>,
    /// The module responsible for delaying the release of processed messages
    /// that have not been fully decapsulated.
    release_delayer: SessionProcessedMessageDelayer<Box<dyn Stream<Item = Round>>, Rng>,
    round_clock_task: AbortHandle,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
}

impl<SessionClock, Rng> MessageScheduler<SessionClock, Rng> {
    pub fn new(session_clock: SessionClock, settings: Settings) -> Self {
        let mut round_clock = IntervalStream::new(interval(settings.round_duration))
            .enumerate()
            .map(|(round, _)| (round as u128).into());

        // Create a broadcast channel
        let (tx, _) = broadcast::channel(2);

        // Spawn a task that sends ticks to the broadcast channel
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let handle = tokio::spawn(Abortable::new(
            async move {
                loop {
                    let next_clock = round_clock.next().await;
                    let _ = tx.send(next_clock);
                }
            },
            abort_registration,
        ));

        // TODO: 1) Move this to a different type that implements `Clone` and `Stream`.
        // TODO: 2) Make it a nice wrapper and move it somewhere else in some utils
        // Create two independent receivers
        let mut stream1 = Box::new(
            BroadcastStream::new(tx.subscribe()).map(|i| (i.unwrap().unwrap() as u128).into()),
        ) as Box<dyn Stream<Item = Round>>;
        let mut stream2 = Box::new(
            BroadcastStream::new(tx.subscribe()).map(|i| (i.unwrap().unwrap() as u128).into()),
        ) as Box<dyn Stream<Item = Round>>;

        let mut rng = ChaCha20Rng::from_entropy();

        let cover_scheduler = SessionCoverTraffic::new(
            crate::cover_traffic_2::Settings {
                additional_safety_intervals: 0,
                expected_intervals_per_session: 1.try_into().unwrap(),
                rounds_per_interval: 1.try_into().unwrap(),
                starting_quota: 1,
            },
            &mut rng,
            stream1,
        );

        let delayer = SessionProcessedMessageDelayer::<Box<dyn Stream<Item = Round>>, Rng>::new(
            crate::release_delayer::Settings {
                maximum_release_delay_in_rounds: 1.try_into().unwrap(),
            },
            rng,
            stream2,
        );

        Self {
            cover_traffic: cover_scheduler,
            release_delayer: delayer,
            round_clock_task: abort_handle,
            session_clock,
            settings,
        }
    }

    /// Notify the cover message submodule that a new data message has been
    /// generated in this session, which will reduce the number of cover
    /// messages generated going forward.
    pub const fn notify_new_data_message(&mut self) {
        self.cover_traffic.notify_new_data_message();
    }

    /// Add a new processed message to the release delayer component queue, for
    /// release during the next release window.
    pub fn schedule_message(&mut self, message: OutboundMessage) {
        self.release_delayer.schedule_message(message);
    }
}

impl<SessionClock, Rng> Stream for MessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone + Unpin,
{
    type Item = RoundInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            cover_traffic,
            release_delayer,
            round_clock,
            session_clock,
            settings,
        } = &mut *self;

        // We update session info on new sessions.
        match session_clock.poll_next_unpin(cx) {
            Poll::Ready(Some(new_session_info)) => {
                // TODO: Update `round_clock` to be able to be cloned an passed to the
                // sub-streams.
                setup_new_session(
                    cover_traffic,
                    release_delayer,
                    round_clock,
                    settings,
                    &new_session_info,
                );
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        // We do not return anything if a new round has not elapsed.
        match round_clock.poll_next_unpin(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Ready(Some(())) => {}
        }
        trace!(target: LOG_TARGET, "New round started.");

        // Sub-streams should never get out of sync. We check that in debug builds to
        // catch any issues early on.
        debug_assert!(
            cover_traffic.current_round() == release_delayer.current_round(),
            "The two sub-streams should never get out of sync."
        );
        // We poll the sub-stream and return the right result accordingly.
        let cover_traffic_output = cover_traffic.poll_next_round().map(|()| vec![].into());
        let release_delayer_output = release_delayer.poll_next_round();

        match (cover_traffic_output, release_delayer_output) {
            // If none of the sub-streams is ready, we do not return anything.
            (None, None) => {
                // We consumed the round clock, so we need to wake the awaker to get triggered
                // on the next round again.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            // If at least one sub-stream yields a result, we return a new element, which might also
            // contain no elements if the cover message module did not yield a new cover message and
            // there were no queue messages to be released in this release window.
            (cover_message, processed_messages) => Poll::Ready(Some(RoundInfo {
                cover_message,
                processed_messages: processed_messages.unwrap_or_default(),
            })),
        }
    }
}

/// Reset the sub-streams providing the new session info and the round clock at
/// the beginning of a new session.
fn setup_new_session<Rng, RoundClock>(
    cover_traffic: &mut SessionCoverTraffic<RoundClock>,
    release_delayer: &mut SessionProcessedMessageDelayer<RoundClock, Rng>,
    round_clock: &mut RoundClock,
    settings: &Settings,
    new_session_info: &SessionInfo,
) where
    Rng: rand::Rng + Clone,
{
    trace!(target: LOG_TARGET, "New session {} started with session info: {new_session_info:?}", new_session_info.session_number);
    *cover_traffic = settings.cover_traffic(new_session_info.core_quota, &mut release_delayer.rng);
    *release_delayer = settings.release_delayer(release_delayer.rng.clone());
    *round_clock = settings.round_clock();
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub additional_safety_intervals: usize,
    pub expected_intervals_per_session: NonZeroUsize,
    pub maximum_release_delay_in_rounds: NonZeroUsize,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroUsize,
}

impl Settings {
    fn cover_traffic<RoundClock, Rng>(
        &self,
        core_quota: usize,
        rng: &mut Rng,
        round_clock: RoundClock,
    ) -> SessionCoverTraffic<RoundClock>
    where
        Rng: rand::Rng,
    {
        SessionCoverTraffic::new(
            crate::cover_traffic_2::Settings {
                additional_safety_intervals: self.additional_safety_intervals,
                expected_intervals_per_session: self.expected_intervals_per_session,
                rounds_per_interval: self.rounds_per_interval,
                starting_quota: core_quota,
            },
            rng,
            round_clock,
        )
    }

    fn release_delayer<RoundClock, Rng>(
        &self,
        rng: Rng,
        round_clock: RoundClock,
    ) -> SessionProcessedMessageDelayer<RoundClock, Rng>
    where
        Rng: rand::Rng,
    {
        SessionProcessedMessageDelayer::new(
            crate::release_delayer::Settings {
                maximum_release_delay_in_rounds: self.maximum_release_delay_in_rounds,
            },
            rng,
            round_clock,
        )
    }
}

#[cfg(test)]
mod tests {
    use core::{
        future::Future as _,
        num::NonZeroUsize,
        task::{Context, Poll},
        time::Duration,
    };
    use std::collections::HashSet;

    use futures::{task::noop_waker_ref, Stream, StreamExt as _};
    use rand::SeedableRng as _;
    use rand_chacha::ChaCha20Rng;
    use tokio::time::{interval, interval_at, sleep, Instant};
    use tokio_stream::wrappers::IntervalStream;

    use crate::{
        cover_traffic_2::SessionCoverTraffic,
        message::OutboundMessage,
        message_scheduler::{
            round_info::RoundInfo, session_info::SessionInfo, MessageScheduler, Settings,
        },
        release_delayer::SessionProcessedMessageDelayer,
    };

    fn default_scheduler(
        session_duration: Duration,
        round_duration: Duration,
    ) -> MessageScheduler<Box<dyn Stream<Item = SessionInfo> + Unpin>, ChaCha20Rng> {
        let rng = ChaCha20Rng::from_entropy();
        MessageScheduler {
            // No scheduled messages.
            cover_traffic: SessionCoverTraffic::with_test_values(0, HashSet::new(), 0),
            // No messages to release at this round.
            release_delayer: SessionProcessedMessageDelayer::with_test_values(
                0,
                NonZeroUsize::new(1).unwrap(),
                // Next scheduled release is 2 rounds in the future.
                2,
                rng,
                vec![],
            ),
            round_clock: Box::new(IntervalStream::new(interval(round_duration)).map(|_| ())),
            session_clock: Box::new(
                // First session is assumed to have already started, so we delay the start to the
                // next session slot.
                IntervalStream::new(interval_at(
                    Instant::now() + session_duration,
                    session_duration,
                ))
                .enumerate()
                .map(|(iteration, _)| SessionInfo {
                    // First iteration is already considered started in the tests, so first clock
                    // brings session `1` alive.
                    session_number: u128::try_from(iteration + 1).unwrap().into(),
                    core_quota: 1,
                }),
            ),
            // Not relevant for tests.
            settings: Settings {
                additional_safety_intervals: 0,
                expected_intervals_per_session: NonZeroUsize::new(2).unwrap(),
                maximum_release_delay_in_rounds: NonZeroUsize::new(1).unwrap(),
                round_duration,
                rounds_per_interval: NonZeroUsize::new(2).unwrap(),
            },
        }
    }

    #[tokio::test]
    async fn no_substream_ready() {
        let mut scheduler = default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
        let mut cx = Context::from_waker(noop_waker_ref());

        // We poll for round 0, which returns `Pending`, as per the default scheduler
        // configuration.
        sleep(Duration::from_millis(100)).await;
        assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
        // We sleep for a bit more than one round.
        sleep(Duration::from_millis(600)).await; // We poll for round 1, which returns `Pending`.
        assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
    }

    #[tokio::test]
    async fn cover_traffic_substream_ready() {
        // Round 0 contains a cover message.
        let mut scheduler = {
            let mut scheduler =
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
            scheduler.cover_traffic =
                SessionCoverTraffic::with_test_values(0, HashSet::from([0]), 0);

            scheduler
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        // Poll for round 0, which should return a cover message as per override over
        // the default.
        sleep(Duration::from_millis(100)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(
            poll_result,
            Poll::Ready(Some(RoundInfo {
                cover_message: Some(vec![].into()),
                processed_messages: vec![]
            }))
        );
    }

    #[tokio::test]
    async fn release_delayer_substream_ready() {
        // Round 0 contains processed messages.
        let mut scheduler = {
            let mut scheduler =
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                0,
                NonZeroUsize::new(5).unwrap(),
                0,
                ChaCha20Rng::from_entropy(),
                vec![OutboundMessage::from(b"test".to_vec())],
            );

            scheduler
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        // Poll for round 0, which should return the processed messages as per override
        // over the default.
        sleep(Duration::from_millis(100)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(
            poll_result,
            Poll::Ready(Some(RoundInfo {
                cover_message: None,
                processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
            }))
        );
    }

    #[tokio::test]
    async fn both_substreams_ready() {
        // Round 0 contains both a cover message and processed messages.
        let mut scheduler = {
            let mut scheduler =
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                0,
                NonZeroUsize::new(5).unwrap(),
                0,
                ChaCha20Rng::from_entropy(),
                vec![OutboundMessage::from(b"test".to_vec())],
            );
            scheduler.cover_traffic =
                SessionCoverTraffic::with_test_values(0, HashSet::from([0]), 0);

            scheduler
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        // Poll for round 0, which should return the processed messages and a cover
        // message as per override over the default.
        sleep(Duration::from_millis(100)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(
            poll_result,
            Poll::Ready(Some(RoundInfo {
                cover_message: Some(vec![].into()),
                processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
            }))
        );
    }

    #[tokio::test]
    async fn round_change() {
        // Round 2 contains a cover message and round 2 contains a processed message.
        let mut scheduler = {
            let mut scheduler =
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                0,
                NonZeroUsize::new(5).unwrap(),
                2,
                ChaCha20Rng::from_entropy(),
                vec![OutboundMessage::from(b"test".to_vec())],
            );
            scheduler.cover_traffic =
                SessionCoverTraffic::with_test_values(0, HashSet::from([1]), 0);

            scheduler
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        // Poll for round 0, which should return `Pending`.
        sleep(Duration::from_millis(100)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(poll_result, Poll::Pending);

        // Poll for round 1, which should return a cover message.
        sleep(Duration::from_millis(500)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(
            poll_result,
            Poll::Ready(Some(RoundInfo {
                cover_message: Some(vec![].into()),
                processed_messages: vec![]
            }))
        );

        // Poll for round 2, which should return the processed messages.
        sleep(Duration::from_millis(500)).await;
        let poll_result = scheduler.poll_next_unpin(&mut cx);
        assert_eq!(
            poll_result,
            Poll::Ready(Some(RoundInfo {
                cover_message: None,
                processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
            }))
        );
    }

    #[tokio::test]
    async fn session_change() {
        let mut scheduler = {
            let mut scheduler =
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500));
            // We set the number of processed messages to `1`, so we can test the value is
            // reset on session changes.
            scheduler.cover_traffic = SessionCoverTraffic::with_test_values(0, HashSet::new(), 1);
            // We override the queue of unreleased messages, so we can test the value is
            // reset on session changes.
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                0,
                NonZeroUsize::new(5).unwrap(),
                2,
                ChaCha20Rng::from_entropy(),
                vec![OutboundMessage::from(b"test".to_vec())],
            );

            scheduler
        };
        assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 1);
        assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 1);
        let mut cx = Context::from_waker(noop_waker_ref());

        // Poll after new session. All the sub-streams should be reset.
        let mut waiting_time_fut = Box::pin(sleep(Duration::from_millis(3_500)));
        while waiting_time_fut.as_mut().poll(&mut cx).is_pending() {
            // We simulate polling the scheduler while waiting for the session to change.
            let _ = scheduler.poll_next_unpin(&mut cx);
            sleep(Duration::from_millis(500)).await;
        }

        assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 0);
        assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 0);
    }
}
