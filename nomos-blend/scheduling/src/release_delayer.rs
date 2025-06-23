use core::{mem, num::NonZeroUsize};

use tracing::{debug, trace};

use crate::message::OutboundMessage;

const LOG_TARGET: &str = "blend::scheduling::delay";

/// A scheduler for delaying processed messages and batch them into release
/// rounds, as per the specification.
pub struct SessionReleaseDelayer<Rng> {
    /// The current round within the session the scheduler is at.
    current_round: usize,
    /// The maximum delay, in terms of rounds, between two consecutive release
    /// rounds.
    maximum_release_delay_in_rounds: NonZeroUsize,
    /// The next scheduled round to release the queued messages.
    next_release_round: usize,
    /// The random generator to select rounds for message releasing.
    pub rng: Rng,
    /// The messages temporarily queued by the scheduler in between release
    /// rounds.
    unreleased_messages: Vec<OutboundMessage>,
}

impl<Rng> SessionReleaseDelayer<Rng>
where
    Rng: rand::Rng,
{
    /// Initialize the scheduler with the required details.
    pub fn new(
        Settings {
            maximum_release_delay_in_rounds,
        }: Settings,
        mut rng: Rng,
    ) -> Self {
        debug!(target: LOG_TARGET, "Creating new release delayer with {maximum_release_delay_in_rounds} maximum delay (in rounds).");
        let next_release_round =
            schedule_next_release_round_offset(maximum_release_delay_in_rounds, &mut rng);
        Self {
            current_round: 0,
            maximum_release_delay_in_rounds,
            next_release_round: next_release_round.get(),
            rng,
            unreleased_messages: Vec::new(),
        }
    }

    /// Poll the scheduler for a potential new release round, while also
    /// advancing its internal round counter. Rounds follow 0-based indexing, so
    /// the very first round in a session is considered to be `0`.
    ///
    /// **This function is meant to be externally called at every round, as
    /// failing to do so might result in missed or delayed release rounds**.
    pub fn poll_next_round(&mut self) -> Option<Vec<OutboundMessage>> {
        let current_round = self.current_round;
        // We can safely saturate, since even if we ever reach the end and there's a
        // scheduled message, it will be consumed the first time the value is hit.
        self.current_round = self.current_round.saturating_add(1);

        // If it's time to release messages...
        if current_round == self.next_release_round {
            // Reset the next release round.
            self.next_release_round = current_round
                .checked_add(
                    schedule_next_release_round_offset(
                        self.maximum_release_delay_in_rounds,
                        &mut self.rng,
                    )
                    .get(),
                )
                .expect("Overflow when scheduling next release round offset.");
            // Return the unreleased messages and clear the buffer.
            let messages = mem::take(&mut self.unreleased_messages);
            debug!(target: LOG_TARGET, "Round {current_round} is a release round.");
            return Some(messages);
        }
        trace!(target: LOG_TARGET, "Round {current_round} is not a release round.");
        None
    }
}

impl<Rng> SessionReleaseDelayer<Rng> {
    pub const fn current_round(&self) -> usize {
        self.current_round
    }

    #[cfg(test)]
    pub const fn with_test_values(
        current_round: usize,
        maximum_release_delay_in_rounds: NonZeroUsize,
        next_release_round: usize,
        rng: Rng,
        unreleased_messages: Vec<OutboundMessage>,
    ) -> Self {
        Self {
            current_round,
            maximum_release_delay_in_rounds,
            next_release_round,
            rng,
            unreleased_messages,
        }
    }

    #[cfg(test)]
    pub fn unreleased_messages(&self) -> &[OutboundMessage] {
        &self.unreleased_messages
    }
}

impl<Rng> SessionReleaseDelayer<Rng> {
    /// Add a new message to the queue to be released at the next release round
    /// along with any other queued message.
    pub fn schedule_message(&mut self, message: OutboundMessage) {
        self.unreleased_messages.push(message);
    }
}

/// Calculates the random offset, in the range [1, `max`], from the current
/// round as the next release round.
fn schedule_next_release_round_offset<Rng>(
    maximum_release_delay_in_rounds: NonZeroUsize,
    rng: &mut Rng,
) -> NonZeroUsize
where
    Rng: rand::Rng,
{
    NonZeroUsize::try_from(rng.gen_range(1..=maximum_release_delay_in_rounds.get()))
        .expect("Randomly generated offset should be strictly positive.")
}

/// The settings to initialize the release delayer scheduler.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The max number of rounds to wait between two release rounds, as per the
    /// specification.
    pub maximum_release_delay_in_rounds: NonZeroUsize,
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroUsize;

    use rand::SeedableRng as _;
    use rand_chacha::ChaCha20Rng;

    use crate::{message::OutboundMessage, release_delayer::SessionReleaseDelayer};

    #[test]
    fn poll_none_on_unscheduled_round() {
        let mut delayer = SessionReleaseDelayer {
            current_round: 0,
            maximum_release_delay_in_rounds: NonZeroUsize::new(3).expect("Non-zero usize"),
            // Next round is 1, so first call to `poll_next_round` will return `None`.
            next_release_round: 1,
            rng: ChaCha20Rng::from_entropy(),
            unreleased_messages: vec![],
        };
        assert!(delayer.poll_next_round().is_none());
        // Check that current round has been incremented.
        assert_eq!(delayer.current_round, 1);
        // Check that next release round has not changed.
        assert_eq!(delayer.next_release_round, 1);
    }

    #[test]
    fn poll_empty_on_scheduled_round_with_empty_queue() {
        let mut delayer = SessionReleaseDelayer {
            current_round: 1,
            maximum_release_delay_in_rounds: NonZeroUsize::new(3).expect("Non-zero usize"),
            // Next round is 1, so next call to `poll_next_round` will return `Some`.
            next_release_round: 1,
            rng: ChaCha20Rng::from_entropy(),
            unreleased_messages: vec![],
        };
        assert_eq!(delayer.poll_next_round(), Some(vec![]));
        // Check that current round has been incremented.
        assert_eq!(delayer.current_round, 2);
        // Check that next release round has been updated with an offset in the
        // expected range [1, 3], so the new value must be in the range [2, 4].
        assert!((2..=4).contains(&delayer.next_release_round));
    }

    #[test]
    fn poll_non_empty_on_scheduled_round_with_non_empty_queue() {
        let mut delayer = SessionReleaseDelayer {
            current_round: 1,
            maximum_release_delay_in_rounds: NonZeroUsize::new(3).expect("Non-zero usize"),
            // Next round is 1, so next call to `poll_next_round` will return `Some`.
            next_release_round: 1,
            rng: ChaCha20Rng::from_entropy(),
            unreleased_messages: vec![OutboundMessage::from(b"test".to_vec())],
        };
        assert_eq!(
            delayer.poll_next_round(),
            Some(vec![OutboundMessage::from(b"test".to_vec())])
        );
        // Check that current round has been incremented.
        assert_eq!(delayer.current_round, 2);
        // Check that next release round has been updated with an offset in the
        // expected range [1, 3], so the new value must be in the range [2, 4].
        assert!((2..=4).contains(&delayer.next_release_round));
    }

    #[test]
    fn add_message_to_queue() {
        let mut delayer = SessionReleaseDelayer {
            current_round: 1,
            maximum_release_delay_in_rounds: NonZeroUsize::new(3).expect("Non-zero usize"),
            next_release_round: 1,
            rng: ChaCha20Rng::from_entropy(),
            unreleased_messages: vec![],
        };
        delayer.schedule_message(OutboundMessage::from(b"test".to_vec()));
        // Check that the new message was added to the queue.
        assert_eq!(
            delayer.unreleased_messages,
            vec![OutboundMessage::from(b"test".to_vec())]
        );
    }
}
