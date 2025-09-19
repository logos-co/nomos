use std::{
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt as _;
use groth16::Field as _;
use nomos_core::crypto::ZkHash;
use tokio::time::{sleep, timeout, Sleep};

use crate::message_blend::{PrivateInfo, PublicInfo, SessionInfo};

/// A staging type that initializes a [`SessionEventStream`] by consuming
/// the first [`Session`] from the underlying stream, expected to be yielded
/// within a short timeout.
pub struct UninitializedSessionEventStream<Stream> {
    stream: FirstReadyStream<Stream>,
    transition_period: Duration,
}

impl<Stream> UninitializedSessionEventStream<Stream> {
    #[must_use]
    pub const fn new(
        session_stream: Stream,
        first_ready_timeout: Duration,
        transition_period: Duration,
    ) -> Self {
        Self {
            stream: FirstReadyStream::new(session_stream, first_ready_timeout),
            transition_period,
        }
    }
}

impl<Stream, Session> UninitializedSessionEventStream<Stream>
where
    Stream: futures::Stream<Item = Session> + Unpin,
{
    /// Initializes a [`SessionEventStream`] by consuming the first [`Session`]
    /// from the underlying stream.
    ///
    /// It returns the first [`Session`] and the initialized
    /// [`SessionEventStream`].
    /// It returns an error if the first session is not yielded within the
    /// configured timeout.
    pub async fn await_first_ready(
        self,
    ) -> Result<(Session, SessionEventStream<Stream>), FirstReadyStreamError> {
        let (first_session, remaining_stream) = self.stream.first().await?;
        Ok((
            first_session,
            SessionEventStream::new(remaining_stream, self.transition_period),
        ))
    }
}

#[derive(Clone)]
pub enum SessionEvent<Session> {
    NewSession(Session, SessionInfo),
    TransitionPeriodExpired,
}

/// A stream that alternates between yielding [`SessionEvent::NewSession`]
/// and [`SessionEvent::TransitionPeriodExpired`].
///
/// It wraps a stream of [`Session`]s and yields a [`SessionEvent::NewSession`]
/// as soon as a new [`Session`] is available from the inner stream.
/// Then, it yields a [`SessionEvent::TransitionPeriodExpired`] after
/// the transition period has elapsed.
///
/// # Stream Timeline
/// ```text
/// event stream  : O--E-------O--E--------------O--E-------
/// session stream: |----S1----|--------S2-------|----S3----
///
/// (O: NewSession, E: TransitionPeriodExpired, S*: Sessions)
/// ```
pub struct SessionEventStream<Stream> {
    session_stream: Stream,
    transition_period: Duration,
    transition_period_timer: Option<Pin<Box<Sleep>>>,
}

impl<Stream> SessionEventStream<Stream> {
    #[must_use]
    const fn new(session_stream: Stream, transition_period: Duration) -> Self {
        Self {
            session_stream,
            transition_period,
            transition_period_timer: None,
        }
    }
}

// TODO: Replace this with the actual values after we get the rest of the
// codebase to compile.
const fn mock_session_info() -> SessionInfo {
    SessionInfo {
        public: PublicInfo {
            core_quota: 0,
            core_root: ZkHash::ZERO,
            leader_quota: 0,
            pol_epoch_nonce: ZkHash::ZERO,
            pol_ledger_aged: ZkHash::ZERO,
            session: 0,
            total_stake: 0,
        },
        private: PrivateInfo {
            aged_path: vec![],
            aged_selector: vec![],
            core_path: vec![],
            core_path_selectors: vec![],
            core_sk: ZkHash::ZERO,
            note_value: 0,
            output_number: 0,
            pol_secret_key: ZkHash::ZERO,
            slot: 0,
            slot_secret: ZkHash::ZERO,
            slot_secret_path: vec![],
            starting_slot: 0,
            transaction_hash: ZkHash::ZERO,
        },
    }
}

impl<Stream, Session> futures::Stream for SessionEventStream<Stream>
where
    Stream: futures::Stream<Item = Session> + Unpin,
{
    type Item = SessionEvent<Session>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check if a new session is available.
        match self.session_stream.poll_next_unpin(cx) {
            Poll::Ready(Some(session)) => {
                // Start the transition period timer, and yield the new session.
                // If the previous transition period timer has not been expired yet,
                // it will be overwritten.
                self.transition_period_timer = Some(Box::pin(sleep(self.transition_period)));
                return Poll::Ready(Some(SessionEvent::NewSession(session, mock_session_info())));
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        // Check if the transition period has expired.
        if let Some(timer) = &mut self.transition_period_timer {
            if timer.as_mut().poll(cx).is_ready() {
                self.transition_period_timer = None;
                return Poll::Ready(Some(SessionEvent::TransitionPeriodExpired));
            }
        }

        Poll::Pending
    }
}

/// A stream wrapper that expects the underlying stream to yield its first item
/// within the given timeout.
///
/// Instead of implementing [`futures::Stream`], this provides only
/// [`Self::first`] that explicitly tries to read the first item and returns a
/// result.
struct FirstReadyStream<Stream> {
    stream: Stream,
    timeout: Duration,
}

impl<Stream> FirstReadyStream<Stream> {
    const fn new(stream: Stream, timeout: Duration) -> Self {
        Self { stream, timeout }
    }
}

impl<Stream, Item> FirstReadyStream<Stream>
where
    Stream: futures::Stream<Item = Item> + Unpin,
{
    /// Yields the first item if it is yielded within the timeout from the
    /// underlying stream.
    /// The remaining stream is also returned for continued use.
    async fn first(mut self) -> Result<(Item, Stream), FirstReadyStreamError> {
        let item = timeout(self.timeout, self.stream.next())
            .await
            .map_err(|_| FirstReadyStreamError::FirstItemNotReady)?
            .ok_or(FirstReadyStreamError::StreamClosed)?;
        Ok((item, self.stream))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FirstReadyStreamError {
    #[error("The first item was not yielded in time")]
    FirstItemNotReady,
    #[error("The underlying stream was closed before yielding the first item")]
    StreamClosed,
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use tokio::time::{interval, interval_at, Instant};
    use tokio_stream::wrappers::IntervalStream;

    use super::*;

    #[tokio::test]
    async fn yield_two_events_alternately() {
        let session_duration = Duration::from_secs(1);
        let transition_period = Duration::from_millis(100);
        let time_tolerance = Duration::from_millis(50);

        let mut stream = SessionEventStream::new(
            Box::pin(IntervalStream::new(interval(session_duration))),
            transition_period,
        );

        // NewSession should be emitted immediately.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_, _))
        ));
        let elapsed = start_time.elapsed();
        let tolerance = Duration::from_millis(5);
        assert!(elapsed <= tolerance, "elapsed:{elapsed:?}");

        // TransitionEnd should be emitted after transition_period.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::TransitionPeriodExpired)
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(transition_period) <= time_tolerance,
            "elapsed:{elapsed:?}, expected:{transition_period:?}",
        );

        // NewSession should be emitted after session_duration - transition_period.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_, _))
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(session_duration - transition_period) <= time_tolerance,
            "elapsed:{elapsed:?}, expected:{:?}",
            session_duration - transition_period
        );

        // TransitionEnd should be emitted after transition_period.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::TransitionPeriodExpired)
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(transition_period) <= time_tolerance,
            "elapsed:{elapsed:?}, expected:{transition_period:?}",
        );
    }

    #[tokio::test]
    async fn transition_period_shorter_than_session() {
        let session_duration = Duration::from_millis(500);
        let transition_period = Duration::from_millis(600);
        let time_tolerance = Duration::from_millis(50);

        let mut stream = SessionEventStream::new(
            Box::pin(IntervalStream::new(interval(session_duration))),
            transition_period,
        );

        // NewSession should be emitted immediately.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_, _))
        ));
        let elapsed = start_time.elapsed();
        assert!(elapsed <= time_tolerance, "elapsed:{elapsed:?}");

        // NewSession should be emitted again after session_duration.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_, _))
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(session_duration) <= time_tolerance,
            "elapsed:{elapsed:?}, expected:{session_duration:?}",
        );
    }

    #[tokio::test]
    async fn first_ready_stream_yields_first_item_immediately() {
        // Use an underlying stream that yields the first item nearly immediately.
        let stream = FirstReadyStream::new(
            IntervalStream::new(interval(Duration::from_secs(1)))
                .enumerate()
                .map(|(i, _)| i),
            Duration::from_millis(100),
        );

        let (first, mut stream) = stream.first().await.expect("first item should be yielded");
        assert_eq!(first, 0);
        // Next items are yielded normally.
        assert_eq!(stream.next().await, Some(1));
        assert_eq!(stream.next().await, Some(2));
    }

    #[tokio::test]
    async fn first_relay_stream_fails_if_first_item_is_not_ready() {
        // Use an underlying stream that yield the first item too late.
        let stream = FirstReadyStream::new(
            IntervalStream::new(interval_at(
                // The first time will be yieled after 2s.
                Instant::now() + Duration::from_secs(2),
                Duration::from_secs(1),
            )),
            Duration::from_millis(100),
        );

        assert!(matches!(
            stream.first().await,
            Err(FirstReadyStreamError::FirstItemNotReady)
        ));
    }
}
