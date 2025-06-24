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
use tokio::{
    sync::broadcast::{self, Receiver, Sender},
    time::interval,
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
            rng,
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

        MessageScheduler::new(session_clock, first_session_info, rng, settings)
    }
}

/// The initialized version of [`UninitializedMessageScheduler`] that is created
/// after the session stream yields its first result.
pub struct MessageScheduler<SessionClock, Rng> {
    /// The module responsible for randomly generated cover messages, given the
    /// allowed session quota and accounting for data messages generated within
    /// the session.
    cover_traffic: SessionCoverTraffic<Box<dyn Stream<Item = Round> + Unpin>>,
    /// The module responsible for delaying the release of processed messages
    /// that have not been fully decapsulated.
    release_delayer: SessionProcessedMessageDelayer<Box<dyn Stream<Item = Round> + Unpin>, Rng>,
    round_clock_stream_sender: Sender<Option<Round>>,
    round_clock: Box<dyn Stream<Item = Round> + Unpin>,
    round_clock_task_abort_handle: AbortHandle,
    /// The input stream that ticks upon a session change.
    session_clock: SessionClock,
    /// The settings to initialize all the required sub-streams.
    settings: Settings,
}

impl<SessionClock, Rng> MessageScheduler<SessionClock, Rng>
where
    Rng: rand::Rng,
{
    pub fn new(
        session_clock: SessionClock,
        initial_session_info: SessionInfo,
        mut rng: Rng,
        settings: Settings,
    ) -> Self {
        let mut round_clock = IntervalStream::new(interval(settings.round_duration))
            .enumerate()
            .map(|(round, _)| (round as u128).into());

        // Create a broadcast channel
        let (round_clock_stream_sender, _) = broadcast::channel(2);
        let round_clock_stream_sender_clone = round_clock_stream_sender.clone();

        // Spawn a task that sends ticks to the broadcast channel
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        tokio::spawn(Abortable::new(
            async move {
                loop {
                    let next_clock: Option<Round> = round_clock.next().await;
                    round_clock_stream_sender_clone
                        .send(next_clock)
                        .expect("Propagating round clock to consumers should not fail");
                }
            },
            abort_registration,
        ));

        Self {
            cover_traffic: instantiate_new_cover_scheduler(
                &mut rng,
                round_clock_stream_sender.subscribe(),
                &settings,
                initial_session_info.core_quota,
            ),
            release_delayer: instantiate_new_message_delayer(
                rng,
                round_clock_stream_sender.subscribe(),
                &settings,
            ),
            round_clock: Box::new(
                BroadcastStream::new(round_clock_stream_sender.subscribe()).map(|round| {
                    round
                        .expect("Round to be `Ok`.")
                        .expect("Round to be `Some`.")
                }),
            ) as Box<dyn Stream<Item = Round> + Unpin>,
            round_clock_stream_sender,
            round_clock_task_abort_handle: abort_handle,
            session_clock,
            settings,
        }
    }
}

fn instantiate_new_cover_scheduler<Rng>(
    rng: &mut Rng,
    round_clock_stream_receiver: Receiver<Option<Round>>,
    settings: &Settings,
    starting_quota: usize,
) -> SessionCoverTraffic<Box<dyn Stream<Item = Round> + Unpin>>
where
    Rng: rand::Rng,
{
    SessionCoverTraffic::new(
        crate::cover_traffic_2::Settings {
            additional_safety_intervals: settings.additional_safety_intervals,
            expected_intervals_per_session: settings.expected_intervals_per_session,
            rounds_per_interval: settings.rounds_per_interval,
            starting_quota,
        },
        rng,
        Box::new(
            BroadcastStream::new(round_clock_stream_receiver).map(|round| {
                round
                    .expect("Round to be `Ok`.")
                    .expect("Round to be `Some`.")
            }),
        ) as Box<dyn Stream<Item = Round> + Unpin>,
    )
}

fn instantiate_new_message_delayer<Rng>(
    rng: Rng,
    round_clock_stream_receiver: Receiver<Option<Round>>,
    settings: &Settings,
) -> SessionProcessedMessageDelayer<Box<dyn Stream<Item = Round> + Unpin>, Rng>
where
    Rng: rand::Rng,
{
    SessionProcessedMessageDelayer::new(
        crate::release_delayer::Settings {
            maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
        },
        rng,
        Box::new(
            BroadcastStream::new(round_clock_stream_receiver).map(|round| {
                round
                    .expect("Round to be `Ok`.")
                    .expect("Round to be `Some`.")
            }),
        ) as Box<dyn Stream<Item = Round> + Unpin>,
    )
}

impl<SessionClock, Rng> MessageScheduler<SessionClock, Rng> {
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

impl<SessionClock, Rng> Drop for MessageScheduler<SessionClock, Rng> {
    fn drop(&mut self) {
        self.round_clock_task_abort_handle.abort();
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
            round_clock_task_abort_handle,
            cover_traffic,
            release_delayer,
            round_clock,
            session_clock,
            settings,
            ..
        } = &mut *self;

        // We update session info on new sessions.
        match session_clock.poll_next_unpin(cx) {
            Poll::Ready(Some(new_session_info)) => {
                setup_new_session(
                    round_clock_task_abort_handle,
                    cover_traffic,
                    release_delayer,
                    settings,
                    &new_session_info,
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
                // We consumed the round clock, so we need to wake the awaker to get triggered
                // on the next round again.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            (Poll::Ready(None), _) | (_, Poll::Ready(None)) => Poll::Ready(None),
            // If at least one sub-stream yields a result, we return a new element, which might also
            // contain no elements if the cover message module did not yield a new cover message and
            // there were no queue messages to be released in this release window.
            (cover_message_ready_output, processed_messages_ready_output) => {
                let cover_message =
                    (cover_message_ready_output == Poll::Ready(Some(()))).then(|| vec![].into());
                let processed_messages = if let Poll::Ready(Some(processed_messages)) =
                    processed_messages_ready_output
                {
                    processed_messages
                } else {
                    vec![]
                };
                Poll::Ready(Some(RoundInfo {
                    processed_messages,
                    cover_message,
                }))
            }
        }
    }
}

/// Reset the sub-streams providing the new session info and the round clock at
/// the beginning of a new session.
fn setup_new_session<Rng>(
    abort_handle: &mut AbortHandle,
    cover_traffic: &mut SessionCoverTraffic<Box<dyn Stream<Item = Round> + Unpin>>,
    release_delayer: &mut SessionProcessedMessageDelayer<
        Box<dyn Stream<Item = Round> + Unpin>,
        Rng,
    >,
    settings: &Settings,
    new_session_info: &SessionInfo,
) where
    Rng: rand::Rng + Clone,
{
    trace!(target: LOG_TARGET, "New session {} started with session info: {new_session_info:?}", new_session_info.session_number);

    abort_handle.abort();
    while !abort_handle.is_aborted() {}

    let mut round_clock = IntervalStream::new(interval(settings.round_duration))
        .enumerate()
        .map(|(round, _)| (round as u128).into());
    // Create a broadcast channel
    let (round_clock_stream_sender, _) = broadcast::channel(2);
    let round_clock_stream_sender_clone = round_clock_stream_sender.clone();

    // Spawn a task that sends ticks to the broadcast channel
    let (new_abort_handle, new_abort_registration) = AbortHandle::new_pair();
    tokio::spawn(Abortable::new(
        async move {
            loop {
                let next_clock: Option<Round> = round_clock.next().await;
                round_clock_stream_sender_clone
                    .send(next_clock)
                    .expect("Propagating round clock to consumers should not fail");
            }
        },
        new_abort_registration,
    ));

    *cover_traffic = instantiate_new_cover_scheduler(
        &mut release_delayer.rng,
        round_clock_stream_sender.subscribe(),
        settings,
        new_session_info.core_quota,
    );
    *release_delayer = instantiate_new_message_delayer(
        release_delayer.rng.clone(),
        round_clock_stream_sender.subscribe(),
        settings,
    );
    *abort_handle = new_abort_handle;
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub additional_safety_intervals: usize,
    pub expected_intervals_per_session: NonZeroUsize,
    pub maximum_release_delay_in_rounds: NonZeroUsize,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroUsize,
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

    use futures::{stream::empty, task::noop_waker_ref, Stream, StreamExt as _};
    use rand::SeedableRng as _;
    use rand_chacha::ChaCha20Rng;
    use tokio::time::{interval_at, sleep, Instant};
    use tokio_stream::{iter, wrappers::IntervalStream};

    use crate::{
        cover_traffic_2::SessionCoverTraffic,
        message::OutboundMessage,
        message_scheduler::{
            round_info::{Round, RoundInfo},
            session_info::SessionInfo,
            MessageScheduler, Settings,
        },
        release_delayer::SessionProcessedMessageDelayer,
        UninitializedMessageScheduler,
    };

    async fn default_scheduler(
        session_duration: Duration,
        round_duration: Duration,
    ) -> MessageScheduler<Box<dyn Stream<Item = SessionInfo> + Unpin>, ChaCha20Rng> {
        let rng = ChaCha20Rng::from_entropy();
        let session_clock = IntervalStream::new(interval_at(
            Instant::now() + session_duration,
            session_duration,
        ))
        .enumerate()
        .map(|(iteration, _)| SessionInfo {
            // First iteration is already considered started in the tests, so first clock
            // brings session `1` alive.
            session_number: u128::try_from(iteration + 1).unwrap().into(),
            core_quota: 1,
        });

        UninitializedMessageScheduler::new(
            Box::new(session_clock) as Box<dyn Stream<Item = SessionInfo> + Unpin>,
            Settings {
                additional_safety_intervals: 0,
                expected_intervals_per_session: NonZeroUsize::new(2).unwrap(),
                maximum_release_delay_in_rounds: NonZeroUsize::new(1).unwrap(),
                round_duration,
                rounds_per_interval: NonZeroUsize::new(2).unwrap(),
            },
            rng,
        )
        .wait_next_session_start()
        .await
    }

    #[tokio::test]
    async fn no_substream_ready() {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
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
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
            scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
                Box::new(iter([0u128]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
                HashSet::from([0u128.into()]),
                0,
            );

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
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                NonZeroUsize::new(5).unwrap(),
                0u128.into(),
                ChaCha20Rng::from_entropy(),
                Box::new(iter([0u128]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
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
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                NonZeroUsize::new(5).unwrap(),
                0u128.into(),
                ChaCha20Rng::from_entropy(),
                Box::new(iter([0u128]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
                vec![OutboundMessage::from(b"test".to_vec())],
            );
            scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
                Box::new(iter([0u128]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
                HashSet::from([0u128.into()]),
                0,
            );

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
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                NonZeroUsize::new(5).unwrap(),
                2u128.into(),
                ChaCha20Rng::from_entropy(),
                Box::new(iter([1u128, 2]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
                vec![OutboundMessage::from(b"test".to_vec())],
            );
            scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
                Box::new(iter([1u128, 2]).map(Into::into)) as Box<dyn Stream<Item = Round> + Unpin>,
                HashSet::from([1u128.into()]),
                0,
            );

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
                default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
            // We set the number of processed messages to `1`, so we can test the value is
            // reset on session changes.
            scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
                Box::new(empty()) as Box<dyn Stream<Item = Round> + Unpin>,
                HashSet::new(),
                1,
            );
            // We override the queue of unreleased messages, so we can test the value is
            // reset on session changes.
            scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
                NonZeroUsize::new(5).unwrap(),
                2u128.into(),
                ChaCha20Rng::from_entropy(),
                Box::new(empty()) as Box<dyn Stream<Item = Round> + Unpin>,
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
