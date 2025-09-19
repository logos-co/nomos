use std::{num::NonZero, pin::Pin};

use cryptarchia_engine::{
    EpochConfig, Slot,
    time::{SlotConfig, SlotTimer},
};
use futures::StreamExt as _;
use time::OffsetDateTime;
use tokio_stream::wrappers::IntervalStream;

use crate::{EpochSlotTickStream, SlotTick};

pub fn slot_timer(
    slot_config: SlotConfig,
    datetime: OffsetDateTime,
    current_slot: Slot,
    epoch_config: EpochConfig,
    base_period_length: NonZero<u64>,
) -> EpochSlotTickStream {
    Pin::new(Box::new(
        IntervalStream::new(SlotTimer::new(slot_config).slot_interval(datetime))
            .zip(futures::stream::iter(std::iter::successors(
                Some(current_slot + 1), // +1 because `slot_interval` ticks from the next slot
                |&slot| Some(slot + 1),
            )))
            .map(move |(_, slot)| SlotTick {
                epoch: epoch_config.epoch(slot, base_period_length),
                slot,
            }),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_slot_timer() {
        let (mut timer, slot_config) = timer();

        // The first tick will be the next slot after the timer was created.
        let tick = timer.next().await;
        // Calculate the expected slot based on the current time.
        let mut expected_slot =
            Slot::from_offset_and_config(OffsetDateTime::now_utc(), slot_config);
        assert_eq!(tick.unwrap().slot, expected_slot);

        // Slots should increment by 1 for each tick.
        let tick = timer.next().await;
        expected_slot = expected_slot + 1;
        assert_eq!(tick.unwrap().slot, expected_slot);
    }

    fn timer() -> (EpochSlotTickStream, SlotConfig) {
        let now = OffsetDateTime::now_utc();
        let slot_config = SlotConfig {
            slot_duration: Duration::from_secs(1),
            chain_start_time: now,
        };
        (
            slot_timer(
                slot_config,
                now,
                Slot::from(0),
                EpochConfig {
                    epoch_stake_distribution_stabilization: NonZero::new(1).unwrap(),
                    epoch_period_nonce_buffer: NonZero::new(1).unwrap(),
                    epoch_period_nonce_stabilization: NonZero::new(1).unwrap(),
                },
                NonZero::new(1).unwrap(),
            ),
            slot_config,
        )
    }
}
