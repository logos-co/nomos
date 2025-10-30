use core::{
    num::NonZeroU64,
    pin::Pin,
    task::{Context, Poll},
};
use std::collections::HashSet;

use futures::{Stream, StreamExt as _};
use tracing::{debug, trace};

use crate::message_scheduler::round_info::Round;

const LOG_TARGET: &str = "blend::scheduling::cover";

/// A scheduler for cover messages that can yield a new cover message when
/// polled, as per the specification.
pub struct SessionCoverTraffic<RoundClock> {
    /// The clock ticking at the beginning of each new round.
    round_clock: RoundClock,
    /// The pre-computed set of rounds that will result in a cover message
    /// generated.
    scheduled_message_rounds: HashSet<Round>,
}

impl<RoundClock> SessionCoverTraffic<RoundClock> {
    /// Initialize the scheduler with the required details.
    pub fn new<Rng>(
        Settings {
            additional_safety_intervals,
            expected_intervals_per_session,
            rounds_per_interval,
            starting_quota,
        }: Settings,
        rng: &mut Rng,
        round_clock: RoundClock,
    ) -> Self
    where
        Rng: rand::Rng,
    {
        let total_intervals = expected_intervals_per_session
            .get()
            .checked_add(additional_safety_intervals)
            .expect("Overflow when calculating total intervals per session.");
        let total_rounds = total_intervals
            .checked_mul(rounds_per_interval.get())
            .expect("Overflow when calculating total rounds per session.");
        debug!(target: LOG_TARGET, "Creating new cover message scheduler with {total_rounds} total rounds.");

        let scheduled_message_rounds =
            schedule_message_rounds(starting_quota as usize, total_rounds, rng);
        Self {
            round_clock,
            scheduled_message_rounds,
        }
    }

    #[cfg(test)]
    pub const fn with_test_values(
        round_clock: RoundClock,
        scheduled_message_rounds: HashSet<Round>,
    ) -> Self {
        Self {
            round_clock,
            scheduled_message_rounds,
        }
    }
}

impl<RoundClock> Stream for SessionCoverTraffic<RoundClock>
where
    RoundClock: Stream<Item = Round> + Unpin,
{
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let new_round = match self.round_clock.poll_next_unpin(cx) {
            Poll::Ready(Some(new_round)) => new_round,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };

        // If a new cover message is scheduled...
        if self.scheduled_message_rounds.remove(&new_round) {
            debug!(target: LOG_TARGET, "Emitting new cover message for round {new_round}");
            return Poll::Ready(Some(()));
        }
        trace!(target: LOG_TARGET, "Not a pre-scheduled emission for round {new_round}.");
        // Awake to trigger a new round clock tick.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Pre-compute the rounds that will contain a cover message, given the amount
/// of messages to generate and the total number of rounds.
fn schedule_message_rounds<Rng>(
    mut total_message_count: usize,
    total_round_count: u64,
    rng: &mut Rng,
) -> HashSet<Round>
where
    Rng: rand::Rng,
{
    let mut scheduled_message_rounds = HashSet::with_capacity(total_message_count);
    trace!(target: LOG_TARGET, "Generating {total_message_count} cover message slots.");
    // We are running a loop that assumes the sample space is larger than the number
    // of items we need to generate. If this assumption is not enforced, we end up
    // in a infinite loop.
    assert!(
        total_round_count >= total_message_count as u64,
        "Cannot generate more messages than the available sample space. Total rounds to sample from {total_round_count}. Total messages to generate {total_message_count}."
    );

    while total_message_count > 0 {
        let random_round = rng.gen_range(0..total_round_count);
        if scheduled_message_rounds.insert(u128::from(random_round).into()) {
            total_message_count -= 1;
        } else {
            trace!(target: LOG_TARGET, "Random round generation yielded an already occupied round. Retrying...");
        }
    }
    scheduled_message_rounds
}

/// The settings to initialize the cover message scheduler.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The number of intervals to use as the safety buffer in case of longer
    /// sessions, as per the spec.
    pub additional_safety_intervals: u64,
    /// The number of expected intervals per session.
    pub expected_intervals_per_session: NonZeroU64,
    /// The number of rounds per interval.
    pub rounds_per_interval: NonZeroU64,
    /// The maximum number of messages to generate in this session, before
    /// accounting for any generated data messages.
    pub starting_quota: u64,
}

#[cfg(test)]
mod tests {
    use core::task::{Context, Poll};
    use std::collections::HashSet;

    use futures::{StreamExt as _, task::noop_waker_ref};
    use tokio_stream::iter;

    use crate::cover_traffic::SessionCoverTraffic;

    #[tokio::test]
    async fn no_emission_on_unscheduled_round() {
        let mut scheduler = SessionCoverTraffic {
            round_clock: iter([0u128]).map(Into::into),
            scheduled_message_rounds: HashSet::default(),
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        assert_eq!(scheduler.poll_next_unpin(&mut cx), Poll::Pending);
    }

    #[tokio::test]
    async fn emission_on_scheduled_round() {
        let mut scheduler = SessionCoverTraffic {
            round_clock: iter([0u128]).map(Into::into),
            scheduled_message_rounds: HashSet::from_iter([0u128.into()]),
        };
        let mut cx = Context::from_waker(noop_waker_ref());

        assert_eq!(scheduler.poll_next_unpin(&mut cx), Poll::Ready(Some(())));
        // Check that the scheduled round has been removed from the set.
        assert!(!scheduler.scheduled_message_rounds.contains(&0u128.into()));
    }
}
