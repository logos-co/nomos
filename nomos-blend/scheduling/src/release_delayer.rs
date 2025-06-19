use std::{mem, num::NonZeroUsize};

use crate::message::OutboundMessage;

const LOG_TARGET: &str = "blend::scheduling::delay";

pub(super) struct SessionReleaseDelayer<Rng> {
    current_round: usize,
    maximum_release_delay_in_rounds: NonZeroUsize,
    next_release_round: usize,
    rng: Rng,
    unreleased_messages: Vec<OutboundMessage>,
}

impl<Rng> SessionReleaseDelayer<Rng>
where
    Rng: rand::Rng,
{
    pub fn new(
        CreationOptions {
            maximum_release_delay_in_rounds,
            mut rng,
        }: CreationOptions<Rng>,
    ) -> Self {
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
        self.current_round = self
            .current_round
            .checked_add(1)
            .expect("Overflow when incrementing current round.");

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
            return Some(messages);
        }
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

pub(crate) struct CreationOptions<Rng> {
    pub maximum_release_delay_in_rounds: NonZeroUsize,
    pub rng: Rng,
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
