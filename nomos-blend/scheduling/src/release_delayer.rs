use std::{mem, num::NonZeroUsize};

use tracing::{debug, trace};

use crate::message::OutboundMessage;

const LOG_TARGET: &str = "blend::scheduling::delay";

pub struct SessionReleaseDelayer<Rng> {
    current_round: usize,
    maximum_release_delay_in_rounds: NonZeroUsize,
    next_release_round: usize,
    pub rng: Rng,
    unreleased_messages: Vec<OutboundMessage>,
}

impl<Rng> SessionReleaseDelayer<Rng>
where
    Rng: rand::Rng,
{
    pub fn new(
        CreationOptions {
            maximum_release_delay_in_rounds,
        }: CreationOptions,
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
    pub fn schedule_message(&mut self, message: OutboundMessage) {
        self.unreleased_messages.push(message);
    }
}

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

#[derive(Debug, Clone, Copy)]
pub struct CreationOptions {
    pub maximum_release_delay_in_rounds: NonZeroUsize,
}

#[cfg(test)]
mod tests {
    #[test]
    fn poll_none_on_unscheduled_round() {
        unimplemented!();
    }

    #[test]
    fn poll_empty_on_scheduled_round_with_empty_queue() {
        unimplemented!();
    }

    #[test]
    fn poll_non_empty_on_scheduled_round_with_non_empty_queue() {
        unimplemented!();
    }
}
