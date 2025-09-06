use std::{
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use tokio::time::{sleep, Sleep};

#[derive(Clone)]
pub enum SessionEvent<Session> {
    NewSession(Session),
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
pub struct SessionEventStream<Session> {
    session_stream: Pin<Box<dyn futures::Stream<Item = Session> + Send + Sync>>,
    transition_period: Duration,
    transition_period_timer: Option<Pin<Box<Sleep>>>,
}

impl<Session> SessionEventStream<Session> {
    #[must_use]
    pub fn new(
        session_stream: Pin<Box<dyn futures::Stream<Item = Session> + Send + Sync>>,
        transition_period: Duration,
    ) -> Self {
        Self {
            session_stream,
            transition_period,
            transition_period_timer: None,
        }
    }
}

impl<Session> futures::Stream for SessionEventStream<Session> {
    type Item = SessionEvent<Session>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check if a new session is available.
        match self.session_stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(session)) => {
                // Start the transition period timer, and yield the new session.
                // If the previous transition period timer has not been expired yet,
                // it will be overwritten.
                self.transition_period_timer = Some(Box::pin(sleep(self.transition_period)));
                return Poll::Ready(Some(SessionEvent::NewSession(session)));
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

/// A stream wrapper that ensures the first item is yielded immediately.
///
/// # Panics
/// If the first item is not yielded within 1 second, it panics.
pub struct FirstImmediateStream<Stream> {
    /// The underlying stream.
    stream: Stream,
    /// Timeout to tolerate small delays before the first item is yielded from
    /// the underlying stream.
    /// [`None`] after the first item is yielded.
    first_item_timeout: Option<Pin<Box<Sleep>>>,
}

impl<Stream> FirstImmediateStream<Stream> {
    /// Builds a [`FirstImmediateStream`] by initializing the 1s timeout for the
    /// first item.
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            first_item_timeout: Some(Box::pin(sleep(Duration::from_secs(1)))),
        }
    }
}

impl<Stream, Item> futures::Stream for FirstImmediateStream<Stream>
where
    Stream: futures::Stream<Item = Item> + Unpin,
{
    type Item = Item;

    /// Polls the underlying stream for the next item.
    ///
    /// # Panics
    /// If the first item is not yielded within 1 second, it panics.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check if we are still waiting for the first item.
        if self.first_item_timeout.is_some() {
            // Poll the underlying stream to get the first item.
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    // Stop the timeout and return the first item.
                    self.first_item_timeout = None;
                    return Poll::Ready(Some(item));
                }
                Poll::Ready(None) => panic!("Stream closed before the first item"),
                Poll::Pending => {}
            }

            // If the first item wasn't ready, check the timeout.
            match self
                .first_item_timeout
                .as_mut()
                .expect("timeout must exist")
                .as_mut()
                .poll(cx)
            {
                Poll::Ready(()) => panic!("The first item was not yielded within timeout"),
                Poll::Pending => return Poll::Pending,
            }
        }

        // Poll the underlying stream normally after the first item.
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use tokio::time::{interval, interval_at, Instant};
    use tokio_stream::wrappers::IntervalStream;

    use super::*;

    #[tokio::test]
    async fn yield_two_events_alternately() {
        let session_duration = Duration::from_millis(100);
        let transition_period = Duration::from_millis(10);

        let mut stream = SessionEventStream::new(
            Box::pin(IntervalStream::new(interval(session_duration))),
            transition_period,
        );

        // NewSession should be emitted immediately.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_))
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
            elapsed.abs_diff(transition_period) <= tolerance,
            "elapsed:{elapsed:?}, expected:{transition_period:?}",
        );

        // NewSession should be emitted after session_duration - transition_period.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_))
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(session_duration - transition_period) <= tolerance,
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
            elapsed.abs_diff(transition_period) <= tolerance,
            "elapsed:{elapsed:?}, expected:{transition_period:?}",
        );
    }

    #[tokio::test]
    async fn transition_period_shorter_than_session() {
        let session_duration = Duration::from_millis(10);
        let transition_period = Duration::from_millis(20);

        let mut stream = SessionEventStream::new(
            Box::pin(IntervalStream::new(interval(session_duration))),
            transition_period,
        );

        // NewSession should be emitted immediately.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_))
        ));
        let elapsed = start_time.elapsed();
        let tolerance = Duration::from_millis(5);
        assert!(elapsed <= tolerance, "elapsed:{elapsed:?}");

        // NewSession should be emitted again after session_duration.
        let start_time = Instant::now();
        assert!(matches!(
            stream.next().await,
            Some(SessionEvent::NewSession(_))
        ));
        let elapsed = start_time.elapsed();
        assert!(
            elapsed.abs_diff(session_duration) <= tolerance,
            "elapsed:{elapsed:?}, expected:{session_duration:?}",
        );
    }

    #[tokio::test]
    async fn first_immediate_stream_yields_first_item_immediately() {
        // Initialize with an underlying stream that yields the first item (nearly)
        // immediately.
        let mut stream = FirstImmediateStream::new(
            IntervalStream::new(interval(Duration::from_millis(1500)))
                .enumerate()
                .map(|(i, _)| i),
        );

        // The first item is yieled without panic (timeout).
        assert_eq!(stream.next().await, Some(0));
        // Next items are yielded normally.
        assert_eq!(stream.next().await, Some(1));
        assert_eq!(stream.next().await, Some(2));
    }

    #[tokio::test]
    #[should_panic(expected = "The first item was not yielded within timeout")]
    async fn first_immediate_stream_panics_if_first_item_delayed() {
        // Initialize with an underlying stream that doesn't yield the first item
        // immediately.
        let mut stream = FirstImmediateStream::new(IntervalStream::new(interval_at(
            // The first time will be yieled after 2s.
            Instant::now() + Duration::from_secs(2),
            Duration::from_millis(1500),
        )));

        // This should panic due to the timeout.
        stream.next().await;
    }
}
