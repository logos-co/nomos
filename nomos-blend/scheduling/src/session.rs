use std::{
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::Stream;
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
    session_stream: Pin<Box<dyn Stream<Item = Session> + Send + Sync>>,
    transition_period: Duration,
    transition_period_timer: Option<Pin<Box<Sleep>>>,
}

impl<Session> SessionEventStream<Session> {
    #[must_use]
    pub fn new(
        session_stream: Pin<Box<dyn Stream<Item = Session> + Send + Sync>>,
        transition_period: Duration,
    ) -> Self {
        Self {
            session_stream,
            transition_period,
            transition_period_timer: None,
        }
    }
}

impl<Session> Stream for SessionEventStream<Session> {
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

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use tokio::time::{interval, Instant};
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
}
