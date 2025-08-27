use std::{
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::Stream;
use tokio::time::{sleep, Sleep};

pub enum SessionEvent<Session> {
    NewSession(Session),
    TransitionEnd,
}

pub struct SessionEventStream<Session> {
    session_stream: Pin<Box<dyn Stream<Item = Session> + Send + Sync>>,
    transition_period: Duration,
    transition_period_timer: Option<Pin<Box<Sleep>>>,
}

impl<Session> SessionEventStream<Session> {
    #[must_use]
    pub fn new(
        session_stream: impl Stream<Item = Session> + Send + Sync + 'static,
        transition_period: Duration,
    ) -> Self {
        Self {
            session_stream: Box::pin(session_stream),
            transition_period,
            transition_period_timer: None,
        }
    }
}

impl<Session> Stream for SessionEventStream<Session> {
    type Item = SessionEvent<Session>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.session_stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(session)) => {
                self.transition_period_timer = Some(Box::pin(sleep(self.transition_period)));
                return Poll::Ready(Some(SessionEvent::NewSession(session)));
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        if let Some(timer) = &mut self.transition_period_timer {
            if timer.as_mut().poll(cx).is_ready() {
                self.transition_period_timer = None;
                return Poll::Ready(Some(SessionEvent::TransitionEnd));
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
            Some(SessionEvent::TransitionEnd)
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
            Some(SessionEvent::TransitionEnd)
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
