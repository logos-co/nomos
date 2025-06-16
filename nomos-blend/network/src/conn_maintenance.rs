use std::{
    ops::RangeInclusive,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt as _};

/// Counts the number of messages received from a peer during
/// an interval. `interval` is a field that implements [`futures::Stream`] to
/// support both sync and async environments.
pub struct ConnectionMonitor<ConnectionWindowClock> {
    expected_message_range: RangeInclusive<usize>,
    connection_window_clock: ConnectionWindowClock,
    current_window_message_count: usize,
}

/// A result of connection monitoring during an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMonitorOutput {
    Spammy,
    Unhealthy,
    Healthy,
}

impl<ConnectionWindowClock> ConnectionMonitor<ConnectionWindowClock> {
    pub const fn new(connection_window_clock: ConnectionWindowClock) -> Self {
        Self {
            connection_window_clock,
            current_window_message_count: 0,
            expected_message_range: 0..=0,
        }
    }

    /// Record a message received from the peer.
    pub fn record_message(&mut self) {
        self.current_window_message_count = self
            .current_window_message_count
            .checked_add(1)
            .unwrap_or_else(|| {
                tracing::warn!("Skipping recording a message due to overflow");
                self.current_window_message_count
            });
    }

    const fn reset(&mut self, new_expected_message_count_range: RangeInclusive<usize>) {
        self.current_window_message_count = 0;
        self.expected_message_range = new_expected_message_count_range;
    }

    /// Check if the peer is malicious based on the number of messages sent
    const fn is_spammy(&self) -> bool {
        self.current_window_message_count > *self.expected_message_range.end()
    }

    /// Check if the peer is unhealthy based on the number of messages sent
    const fn is_unhealthy(&self) -> bool {
        self.current_window_message_count < *self.expected_message_range.start()
    }
}

impl<ConnectionWindowClock> ConnectionMonitor<ConnectionWindowClock>
where
    ConnectionWindowClock: Stream<Item = RangeInclusive<usize>> + Unpin,
{
    /// Poll the connection monitor to check if the interval has elapsed.
    /// If the interval has elapsed, evaluate the peer's status,
    /// reset the monitor, and return the result as `Poll::Ready`.
    /// If not, return `Poll::Pending`.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionMonitorOutput> {
        let Poll::Ready(Some(new_expected_message_count_range)) =
            self.connection_window_clock.poll_next_unpin(cx)
        else {
            return Poll::Pending;
        };
        let outcome = if self.is_spammy() {
            ConnectionMonitorOutput::Spammy
        } else if self.is_unhealthy() {
            ConnectionMonitorOutput::Unhealthy
        } else {
            ConnectionMonitorOutput::Healthy
        };
        self.reset(new_expected_message_count_range);
        Poll::Ready(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        task::{Context, Poll},
        time::Duration,
    };

    use futures::task::noop_waker;
    use tokio_stream::StreamExt as _;

    use crate::conn_maintenance::{ConnectionMonitor, ConnectionMonitorOutput};

    #[test]
    fn monitor() {
        let mut monitor = ConnectionMonitor::new(futures::stream::iter(std::iter::repeat(1..=2)));
        // We poll once to set the expected number of messages.
        let _ = monitor.poll(&mut Context::from_waker(&noop_waker()));

        // Recording the minimum expected number of messages,
        // expecting the peer to be healthy
        monitor.record_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Healthy)
        );

        // Recording the maximum expected number of messages,
        // expecting the peer to be healthy
        monitor.record_message();
        monitor.record_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Healthy)
        );

        // Recording more than the expected number of messages,
        // expecting the peer to be malicious
        monitor.record_message();
        monitor.record_message();
        monitor.record_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Spammy)
        );

        // Recording less than the expected number of messages (i.e. no message),
        // expecting the peer to be unhealthy
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Unhealthy)
        );
    }

    #[tokio::test]
    async fn monitor_interval() {
        let interval = Duration::from_millis(100);
        let mut monitor = ConnectionMonitor::new(
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval_at(
                tokio::time::Instant::now() + interval,
                interval,
            ))
            .map(|_| 1..=2),
        );

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        assert!(monitor.poll(&mut cx).is_pending());

        tokio::time::sleep(interval).await;
        assert!(monitor.poll(&mut cx).is_ready());
        assert!(monitor.poll(&mut cx).is_pending());
    }
}
