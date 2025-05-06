use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use fixed::types::U57F7;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Counts the number of effective and drop messages received from a peer during
/// an interval. `interval` is a field that implements [`futures::Stream`] to
/// support both sync and async environments.
pub struct ConnectionMonitor {
    settings: ConnectionMonitorSettings,
    interval: Pin<Box<dyn futures::Stream<Item = ()> + Send>>,
    effective_messages: U57F7,
    drop_messages: U57F7,
}

#[serde_as]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ConnectionMonitorSettings {
    /// Time interval to measure/evaluate the number of messages sent by each
    /// peer.
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub interval: Duration,
    /// The number of effective (data or cover) messages that a peer is expected
    /// to send in a given time window.
    ///
    /// If the count is greater than (expected * (1 + `malicious_tolerance`)),
    /// the peer is considered malicious.
    /// If the count is less than (expected * (1 - `unhealthy_tolerance`)), the
    /// peer is considered unhealthy.
    pub expected_effective_messages: U57F7,
    pub effective_message_malicious_tolerance: U57F7,
    pub effective_message_unhealthy_tolerance: U57F7,
    /// The number of drop messages that a peer is expected to send
    /// in a given time window.
    ///
    /// If the count is greater than (expected * (1 + `malicious_tolerance`)),
    /// the peer is considered malicious.
    /// If the count is less than (expected * (1 - `unhealthy_tolerance`)),
    /// the peer is considered unhealthy.
    pub expected_drop_messages: U57F7,
    pub drop_message_malicious_tolerance: U57F7,
    pub drop_message_unhealthy_tolerance: U57F7,
}

/// A result of connection monitoring during an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMonitorOutput {
    Malicious,
    Unhealthy,
    Healthy,
}

impl ConnectionMonitor {
    pub fn new(
        settings: ConnectionMonitorSettings,
        interval: impl futures::Stream<Item = ()> + Send + 'static,
    ) -> Self {
        Self {
            settings,
            interval: Box::pin(interval),
            effective_messages: U57F7::ZERO,
            drop_messages: U57F7::ZERO,
        }
    }

    /// Record an effective message received from the peer.
    pub fn record_effective_message(&mut self) {
        self.effective_messages = Self::record_message(self.effective_messages);
    }

    /// Record a drop effective message received from the peer.
    pub fn record_drop_message(&mut self) {
        self.drop_messages = Self::record_message(self.drop_messages);
    }

    fn record_message(value: U57F7) -> U57F7 {
        value.checked_add(U57F7::ONE).unwrap_or_else(|| {
            tracing::warn!("Skipping recording a message due to overflow");
            value
        })
    }

    /// Poll the connection monitor to check if the interval has elapsed.
    /// If the interval has elapsed, evaluate the peer's status,
    /// reset the monitor, and return the result as `Poll::Ready`.
    /// If not, return `Poll::Pending`.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionMonitorOutput> {
        if self.interval.as_mut().poll_next(cx).is_ready() {
            let outcome = if self.is_malicious() {
                ConnectionMonitorOutput::Malicious
            } else if self.is_unhealthy() {
                ConnectionMonitorOutput::Unhealthy
            } else {
                ConnectionMonitorOutput::Healthy
            };
            self.reset();
            Poll::Ready(outcome)
        } else {
            Poll::Pending
        }
    }

    const fn reset(&mut self) {
        self.effective_messages = U57F7::ZERO;
        self.drop_messages = U57F7::ZERO;
    }

    /// Check if the peer is malicious based on the number of effective and drop
    /// messages sent
    fn is_malicious(&self) -> bool {
        let effective_threshold = self.settings.expected_effective_messages
            * (U57F7::ONE + self.settings.effective_message_malicious_tolerance);
        let drop_threshold = self.settings.expected_drop_messages
            * (U57F7::ONE + self.settings.drop_message_malicious_tolerance);
        self.effective_messages > effective_threshold || self.drop_messages > drop_threshold
    }

    /// Check if the peer is unhealthy based on the number of effective and drop
    /// messages sent
    fn is_unhealthy(&self) -> bool {
        let effective_threshold = self.settings.expected_effective_messages
            * (U57F7::ONE - self.settings.effective_message_unhealthy_tolerance);
        let drop_threshold = self.settings.expected_drop_messages
            * (U57F7::ONE - self.settings.drop_message_unhealthy_tolerance);
        effective_threshold > self.effective_messages || drop_threshold > self.drop_messages
    }
}

#[cfg(test)]
mod tests {
    use futures::task::noop_waker;
    use tokio_stream::StreamExt as _;

    use super::*;

    #[test]
    fn monitor() {
        let mut monitor = ConnectionMonitor::new(
            ConnectionMonitorSettings {
                interval: Duration::from_secs(1),
                expected_effective_messages: U57F7::from_num(2.0),
                effective_message_malicious_tolerance: U57F7::from_num(0.5),
                effective_message_unhealthy_tolerance: U57F7::from_num(0.1),
                expected_drop_messages: U57F7::from_num(1.0),
                drop_message_malicious_tolerance: U57F7::from_num(0.0),
                drop_message_unhealthy_tolerance: U57F7::from_num(0.0),
            },
            futures::stream::iter(std::iter::repeat(())),
        );

        // Recording the expected number of messages,
        // expecting the peer to be healthy
        monitor.record_effective_message();
        monitor.record_effective_message();
        monitor.record_drop_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Healthy)
        );

        // Recording more than the expected number of effective messages,
        // expecting the peer to be malicious
        monitor.record_effective_message();
        monitor.record_effective_message();
        monitor.record_effective_message();
        monitor.record_effective_message();
        monitor.record_drop_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Malicious)
        );

        // Recording less than the expected number of effective messages,
        // expecting the peer to be unhealthy
        monitor.record_effective_message();
        monitor.record_drop_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Unhealthy)
        );

        // Recording more than the expected number of drop messages,
        // expecting the peer to be malicious
        monitor.record_effective_message();
        monitor.record_effective_message();
        monitor.record_drop_message();
        monitor.record_drop_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Malicious)
        );

        // Recording less than the expected number of drop messages,
        // expecting the peer to be unhealthy
        monitor.record_effective_message();
        monitor.record_effective_message();
        assert_eq!(
            monitor.poll(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(ConnectionMonitorOutput::Unhealthy)
        );
    }

    #[tokio::test]
    async fn monitor_interval() {
        let interval = Duration::from_millis(100);
        let mut monitor = ConnectionMonitor::new(
            ConnectionMonitorSettings {
                interval: Duration::from_secs(1),
                expected_effective_messages: U57F7::from_num(2.0),
                effective_message_malicious_tolerance: U57F7::from_num(0.1),
                effective_message_unhealthy_tolerance: U57F7::from_num(0.1),
                expected_drop_messages: U57F7::from_num(1.0),
                drop_message_malicious_tolerance: U57F7::from_num(0.0),
                drop_message_unhealthy_tolerance: U57F7::from_num(0.0),
            },
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval_at(
                tokio::time::Instant::now() + interval,
                interval,
            ))
            .map(|_| ()),
        );

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        assert!(monitor.poll(&mut cx).is_pending());

        tokio::time::sleep(interval).await;
        assert!(monitor.poll(&mut cx).is_ready());
        assert!(monitor.poll(&mut cx).is_pending());
    }
}
