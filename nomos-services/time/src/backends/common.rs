use std::{num::NonZero, pin::Pin};

use cryptarchia_engine::{
    time::{SlotConfig, SlotTimer},
    EpochConfig, Slot,
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
                Some(current_slot),
                |&slot| Some(slot + 1),
            )))
            .map(move |(_, slot)| SlotTick {
                epoch: epoch_config.epoch(slot, base_period_length),
                slot,
            }),
    ))
}
