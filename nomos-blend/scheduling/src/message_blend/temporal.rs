use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Future as _, Stream, StreamExt as _};
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::time;

pub struct TemporalScheduler<Rng> {
    settings: TemporalSchedulerSettings,
    /// Interval in seconds for running the lottery to release a message
    lottery_interval: time::Interval,
    /// To wait a few seconds after running the lottery before releasing the
    /// message. The lottery returns how long to wait before releasing the
    /// message.
    release_timer: Option<Pin<Box<time::Sleep>>>,
    /// local lottery rng
    rng: Rng,
}

impl<Rng> TemporalScheduler<Rng> {
    pub fn new(settings: TemporalSchedulerSettings, rng: Rng) -> Self {
        let lottery_interval = Self::lottery_interval(settings.max_delay);
        Self {
            settings,
            lottery_interval,
            release_timer: None,
            rng,
        }
    }

    /// Create [`time::Interval`] for running the lottery to release a message.
    fn lottery_interval(max_delay: Duration) -> time::Interval {
        time::interval(Duration::from_secs(Self::lottery_interval_seconds(
            max_delay.as_secs(),
        )))
    }

    /// Calculate the interval in seconds for running the lottery.
    /// The lottery interval is half of the maximum delay,
    /// in order to guarantee that the interval between two subsequent message
    /// emissions is at most [`max_delay_seconds`].
    const fn lottery_interval_seconds(max_delay_seconds: u64) -> u64 {
        max_delay_seconds / 2
    }
}

impl<Rng> TemporalScheduler<Rng>
where
    Rng: rand::Rng,
{
    /// Run the lottery to determine the delay before releasing a message.
    /// The delay is in [0, `lottery_interval_seconds`).
    fn run_lottery(&mut self) -> u64 {
        let interval = Self::lottery_interval_seconds(self.settings.max_delay.as_secs());
        self.rng.gen_range(0..interval)
    }
}

impl<Rng> Stream for TemporalScheduler<Rng>
where
    Rng: RngCore + Unpin,
{
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check whether it's time to run a new lottery to determine the delay.
        if self.lottery_interval.poll_tick(cx).is_ready() {
            let delay = self.run_lottery();
            // Set timer to release the message after the delay.
            self.release_timer = Some(Box::pin(time::sleep(Duration::from_secs(delay))));
        }

        // Check whether the release timer is done if it exists.
        if let Some(timer) = self.release_timer.as_mut() {
            if timer.as_mut().poll(cx).is_ready() {
                self.release_timer.take(); // Reset timer after it's done
                return Poll::Ready(Some(()));
            }
        }
        Poll::Pending
    }
}

/// [`TemporalProcessor`] delays messages randomly to hide timing correlation
/// between incoming and outgoing messages from a node.
///
/// See the [`Stream`] implementation below for more details on how it works.
pub struct TemporalProcessor<M, S> {
    // All scheduled messages
    queue: VecDeque<M>,
    scheduler: S,
}

#[serde_with::serde_as]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TemporalSchedulerSettings {
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub max_delay: Duration,
}

impl<M, S> TemporalProcessor<M, S> {
    pub(crate) const fn new(scheduler: S) -> Self {
        Self {
            queue: VecDeque::new(),
            scheduler,
        }
    }
    /// Schedule a message to be released later.
    pub(crate) fn push_message(&mut self, message: M) {
        self.queue.push_back(message);
    }
}

impl<M, S> Stream for TemporalProcessor<M, S>
where
    M: Unpin,
    S: Stream<Item = ()> + Unpin,
{
    type Item = M;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.scheduler.poll_next_unpin(cx).is_ready() {
            if let Some(msg) = self.queue.pop_front() {
                return Poll::Ready(Some(msg));
            }
        }
        Poll::Pending
    }
}

pub struct TemporalStream<WrappedStream, Scheduler>
where
    WrappedStream: Stream,
    Scheduler: Stream<Item = ()>,
{
    processor: TemporalProcessor<WrappedStream::Item, Scheduler>,
    wrapped_stream: WrappedStream,
}

impl<WrappedStream, Scheduler> Stream for TemporalStream<WrappedStream, Scheduler>
where
    WrappedStream: Stream + Unpin,
    WrappedStream::Item: Unpin,
    Scheduler: Stream<Item = ()> + Unpin,
{
    type Item = WrappedStream::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(item)) = self.wrapped_stream.poll_next_unpin(cx) {
            self.processor.push_message(item);
        }
        self.processor.poll_next_unpin(cx)
    }
}
pub trait TemporalProcessorExt<Scheduler>: Stream
where
    Scheduler: Stream<Item = ()>,
{
    fn temporal_stream(self, scheduler: Scheduler) -> TemporalStream<Self, Scheduler>
    where
        Self: Sized,
    {
        TemporalStream {
            processor: TemporalProcessor::<Self::Item, Scheduler>::new(scheduler),
            wrapped_stream: self,
        }
    }
}

impl<T, S> TemporalProcessorExt<S> for T
where
    T: Stream,
    S: Stream<Item = ()>,
{
}
