use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    MessageScheduler,
    message_scheduler::{Settings, round_info::RoundInfo, session_info::SessionInfo},
};

/// A wrapper around a [`MessageScheduler`] that keeps track of how much quota
/// allowance is consumed at each release round.
///
/// The used allowance is used in the core service state recovery logic, so that
/// a node that is shut down and restarted in the same session won't go above
/// its allocated core quota allowance.
pub struct SchedulerWrapper<SessionClock, Rng, ProcessedMessage> {
    /// The inner message scheduler.
    scheduler: MessageScheduler<SessionClock, Rng, ProcessedMessage>,
    /// The amount of core quota consumed by data messages in the current
    /// release round.
    release_round_consumed_quota: u64,
}

impl<SessionClock, Rng, ProcessedMessage> SchedulerWrapper<SessionClock, Rng, ProcessedMessage>
where
    Rng: rand::Rng + Clone,
{
    pub fn new(
        session_clock: SessionClock,
        initial_session_info: SessionInfo,
        rng: Rng,
        settings: Settings,
    ) -> Self {
        Self {
            release_round_consumed_quota: 0,
            scheduler: MessageScheduler::new(session_clock, initial_session_info, rng, settings),
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage> SchedulerWrapper<SessionClock, Rng, ProcessedMessage> {
    // Method overridden from the underlying message scheduler, so that we capture
    // its calls instead of dereferencing them to the underlying scheduler. The rest
    // of the calls are instead dereferenced to the underlying scheduler.
    pub fn notify_new_data_message(&mut self) {
        self.release_round_consumed_quota = self
            .release_round_consumed_quota
            .checked_add(1)
            .expect("Round consumed quota addition overflow.");
        self.scheduler.notify_new_data_message();
    }
}

impl<SessionClock, Rng, ProcessedMessage> Deref
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage>
{
    type Target = MessageScheduler<SessionClock, Rng, ProcessedMessage>;

    fn deref(&self) -> &Self::Target {
        &self.scheduler
    }
}

impl<SessionClock, Rng, ProcessedMessage> DerefMut
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.scheduler
    }
}

/// A wrapper around a [`RoundInfo`] that returns also the amount of core quota
/// used in the current release round.
pub struct RoundInfoAndConsumedQuota<ProcessedMessage> {
    pub info: RoundInfo<ProcessedMessage>,
    pub consumed_quota: u64,
}

impl<SessionClock, Rng, ProcessedMessage> Stream
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone + Unpin,
    ProcessedMessage: Debug + Unpin,
{
    type Item = RoundInfoAndConsumedQuota<ProcessedMessage>;

    // We poll the underlying scheduler, and if it yields a new round, we count how
    // much core quota allowance was "consumed" between data and cover messages for
    // this release round. Then, we reset the counter for the next release
    // round.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.scheduler.poll_next_unpin(cx) {
            Poll::Ready(Some(round_info)) => {
                // Consumed quota for this round equals the number of blended data messages +
                // the number of cover messages (which at the moment can be either `0` or `1`).
                let total_consumed_quota = self
                    .release_round_consumed_quota
                    .checked_add(u64::from(
                        round_info.cover_message_generation_flag.is_some(),
                    ))
                    .expect("Overflow when computing total consumed quota for release round.");
                self.release_round_consumed_quota = 0;
                Poll::Ready(Some(RoundInfoAndConsumedQuota {
                    consumed_quota: total_consumed_quota,
                    info: round_info,
                }))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
