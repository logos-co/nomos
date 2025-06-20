use core::num::NonZeroUsize;
use std::collections::HashSet;

use tracing::{debug, trace};

const LOG_TARGET: &str = "blend::scheduling::cover";

pub struct SessionCoverTraffic {
    current_round: usize,
    scheduled_message_rounds: HashSet<usize>,
    unprocessed_data_messages: usize,
}

impl SessionCoverTraffic {
    pub fn new<Rng>(
        CreationOptions {
            additional_safety_intervals,
            expected_intervals_per_session,
            rounds_per_interval,
            starting_quota,
        }: CreationOptions,
        rng: &mut Rng,
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

        let scheduled_message_rounds = schedule_message_rounds(starting_quota, total_rounds, rng);
        Self {
            current_round: 0,
            scheduled_message_rounds,
            unprocessed_data_messages: 0,
        }
    }

    #[cfg(test)]
    pub const fn with_test_values(
        current_round: usize,
        scheduled_message_rounds: HashSet<usize>,
        unprocessed_data_messages: usize,
    ) -> Self {
        Self {
            current_round,
            scheduled_message_rounds,
            unprocessed_data_messages,
        }
    }

    pub fn current_round(&self) -> usize {
        self.current_round
    }

    pub fn poll_next_round(&mut self) -> Option<()> {
        let current_round = self.current_round;
        // We can safely saturate, since even if we ever reach the end and there's a
        // scheduled message, it will be consumed the first time the value is hit.
        self.current_round = self.current_round.saturating_add(1);

        // If a new cover message is scheduled...
        if self.scheduled_message_rounds.remove(&current_round) {
            // Check if there's any unprocessed data message, and update the counter.
            let Some(new_unprocessed_data_messages) = self.unprocessed_data_messages.checked_sub(1)
            else {
                // If the value was already zero, we emit a new cover message.
                debug!(
                    target: LOG_TARGET, "Emitting new cover message for round {current_round}"
                );
                return Some(());
            };
            // Else, we skip emission and update the unprocessed data message counter.
            self.unprocessed_data_messages = new_unprocessed_data_messages;
            trace!(target: LOG_TARGET, "Skipping message emission because of override by data message.");
            return None;
        }
        trace!(target: LOG_TARGET, "Not a pre-scheduled emission for round {current_round}.");
        None
    }

    pub const fn notify_new_data_message(&mut self) {
        self.unprocessed_data_messages = self
            .unprocessed_data_messages
            .checked_add(1)
            .expect("Overflow when incrementing unprocessed data messages.");
    }
}

fn schedule_message_rounds<Rng>(
    mut total_message_count: usize,
    total_round_count: usize,
    rng: &mut Rng,
) -> HashSet<usize>
where
    Rng: rand::Rng,
{
    let mut scheduled_message_rounds = HashSet::with_capacity(total_message_count);
    trace!(target: LOG_TARGET, "Generating {total_message_count} cover message slots.");

    while total_message_count > 0 {
        let random_round = rng.gen_range(0..total_round_count);
        if scheduled_message_rounds.insert(random_round) {
            total_message_count -= 1;
        } else {
            trace!(target: LOG_TARGET, "Random round generation generated an existing entry. Retrying...");
        }
    }
    scheduled_message_rounds
}

#[derive(Debug, Clone, Copy)]
pub struct CreationOptions {
    pub additional_safety_intervals: usize,
    pub expected_intervals_per_session: NonZeroUsize,
    pub rounds_per_interval: NonZeroUsize,
    pub starting_quota: usize,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::cover_traffic_2::SessionCoverTraffic;

    #[test]
    fn no_emission_on_unscheduled_round() {
        let mut scheduler = SessionCoverTraffic {
            current_round: 0,
            scheduled_message_rounds: HashSet::default(),
            unprocessed_data_messages: 0,
        };
        assert_eq!(scheduler.poll_next_round(), None);
        // Check that current round has been incremented.
        assert_eq!(scheduler.current_round, 1);
    }

    #[test]
    fn emission_on_scheduled_round() {
        let mut scheduler = SessionCoverTraffic {
            current_round: 1,
            scheduled_message_rounds: HashSet::from_iter([1]),
            unprocessed_data_messages: 0,
        };
        assert_eq!(scheduler.poll_next_round(), Some(()));
        // Check that current round has been incremented.
        assert_eq!(scheduler.current_round, 2);
        // Check that the scheduled round has been removed from the set.
        assert!(!scheduler.scheduled_message_rounds.contains(&1));
    }

    #[test]
    fn no_emission_on_scheduled_round_with_unprocessed_message() {
        let mut scheduler = SessionCoverTraffic {
            current_round: 1,
            scheduled_message_rounds: HashSet::from_iter([1]),
            unprocessed_data_messages: 1,
        };
        assert_eq!(scheduler.poll_next_round(), None);
        // Check that current round has been incremented.
        assert_eq!(scheduler.current_round, 2);
        // Check that the scheduled round has been removed from the set.
        assert!(!scheduler.scheduled_message_rounds.contains(&1));
        // Check that the number of processed messages has been decremented.
        assert_eq!(scheduler.unprocessed_data_messages, 0);
    }

    #[test]
    fn notify_new_data_message() {
        let mut scheduler = SessionCoverTraffic {
            current_round: 1,
            scheduled_message_rounds: HashSet::from_iter([1]),
            unprocessed_data_messages: 0,
        };
        scheduler.notify_new_data_message();
        assert_eq!(scheduler.unprocessed_data_messages, 1);
    }
}
