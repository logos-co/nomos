use std::{
    pin::{pin, Pin},
    task::{Context, Poll},
};

use futures::{Stream, StreamExt as _};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PersistentTransmissionSettings {
    /// The maximum number of messages that can be emitted per second
    pub max_emission_frequency: f64,
}

impl Default for PersistentTransmissionSettings {
    fn default() -> Self {
        Self {
            max_emission_frequency: 1.0,
        }
    }
}

/// Transmit scheduled messages with a persistent rate as a stream.
pub struct PersistentTransmissionStream<MsgStream, Scheduler>
where
    MsgStream: Stream,
{
    stream: MsgStream,
    scheduler: Scheduler,
}

impl<MsgStream, Scheduler> PersistentTransmissionStream<MsgStream, Scheduler>
where
    MsgStream: Stream,
    Scheduler: Stream<Item = ()>,
{
    pub const fn new(stream: MsgStream, scheduler: Scheduler) -> Self {
        Self { stream, scheduler }
    }
}

impl<MessageStream, Scheduler> Stream for PersistentTransmissionStream<MessageStream, Scheduler>
where
    MessageStream: Stream + Unpin,
    MessageStream::Item: Clone + Unpin,
    Scheduler: Stream<Item = ()> + Unpin,
{
    type Item = MessageStream::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            ref mut scheduler,
            ref mut stream,
            ..
        } = self.get_mut();
        if pin!(scheduler).poll_next_unpin(cx).is_pending() {
            return Poll::Pending;
        }
        if let Poll::Ready(Some(item)) = pin!(stream).poll_next(cx) {
            Poll::Ready(Some(item))
        } else {
            Poll::Pending
        }
    }
}

pub trait PersistentTransmissionExt<Scheduler>: Stream
where
    Scheduler: Stream<Item = ()>,
{
    fn persistent_transmission(
        self,
        scheduler: Scheduler,
    ) -> PersistentTransmissionStream<Self, Scheduler>
    where
        Self: Sized + Unpin,
    {
        PersistentTransmissionStream::new(self, scheduler)
    }
}

impl<MessageStream, Scheduler> PersistentTransmissionExt<Scheduler> for MessageStream
where
    MessageStream: Stream,
    Scheduler: Stream<Item = ()>,
{
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt as _;
    use tokio::{sync::mpsc, time};
    use tokio_stream::wrappers::IntervalStream;

    use super::*;

    macro_rules! assert_interval {
        ($last_time:expr, $lower_bound:expr, $upper_bound:expr) => {
            let now = time::Instant::now();
            let interval = now.duration_since(*$last_time);

            assert!(
                interval >= $lower_bound,
                "interval {:?} is too short. lower_bound: {:?}",
                interval,
                $lower_bound,
            );
            assert!(
                interval <= $upper_bound,
                "interval {:?} is too long. upper_bound: {:?}",
                interval,
                $upper_bound,
            );

            *$last_time = now;
        };
    }

    #[tokio::test]
    async fn test_persistent_transmission_stream() {
        let (schedule_sender, schedule_receiver) = mpsc::unbounded_channel();
        let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(schedule_receiver);
        let settings = PersistentTransmissionSettings {
            max_emission_frequency: 1.0,
        };
        // Prepare the expected emission interval with tolerance
        let expected_emission_interval =
            Duration::from_secs_f64(1.0 / settings.max_emission_frequency);
        let tolerance = expected_emission_interval / 10; // 10% tolerance
        let lower_bound = expected_emission_interval - tolerance;
        let upper_bound = expected_emission_interval + tolerance;
        // prepare stream
        let mut persistent_transmission_stream: PersistentTransmissionStream<_, _> = stream
            .persistent_transmission(
                IntervalStream::new(time::interval(expected_emission_interval)).map(|_| ()),
            );
        // Messages must be scheduled in non-blocking manner.
        schedule_sender.send(vec![1]).unwrap();
        schedule_sender.send(vec![2]).unwrap();
        schedule_sender.send(vec![3]).unwrap();

        // Check if expected messages are emitted with the expected interval
        assert_eq!(
            persistent_transmission_stream.next().await.unwrap(),
            vec![1]
        );
        let mut last_time = time::Instant::now();

        assert_eq!(
            persistent_transmission_stream.next().await.unwrap(),
            vec![2]
        );
        assert_interval!(&mut last_time, lower_bound, upper_bound);

        assert_eq!(
            persistent_transmission_stream.next().await.unwrap(),
            vec![3]
        );
        assert_interval!(&mut last_time, lower_bound, upper_bound);

        // Schedule a new message and check if it is emitted at the next interval
        schedule_sender.send(vec![4]).unwrap();
        assert_eq!(
            persistent_transmission_stream.next().await.unwrap(),
            vec![4]
        );
        assert_interval!(&mut last_time, lower_bound, upper_bound);
    }
}
