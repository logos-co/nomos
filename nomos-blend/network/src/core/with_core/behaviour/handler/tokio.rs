pub use std::{num::NonZeroU64, ops::RangeInclusive, time::Duration};

pub use nomos_utils::math::NonNegativeF64;
pub use tokio_stream::StreamExt as _;

use crate::core::with_core::behaviour::IntervalStreamProvider;

#[derive(Clone)]
/// Provider of a stream of observation windows used by the Blend connection
/// monitor to evaluate peers.
///
/// At each interval, it returns the [min,max] (inclusive) range of expected
/// messages from the peer, as per the specification.
pub struct ObservationWindowTokioIntervalProvider {
    pub round_duration_seconds: NonZeroU64,
    pub maximal_delay_rounds: NonZeroU64,
    pub blending_ops_per_message: u64,
    pub normalization_constant: NonNegativeF64,
    pub membership_size: NonZeroU64,
    pub rounds_per_observation_window: NonZeroU64,
    pub minimum_messages_coefficient: NonZeroU64,
}

impl ObservationWindowTokioIntervalProvider {
    fn calculate_expected_message_range(&self) -> RangeInclusive<u64> {
        // TODO: Remove unsafe arithmetic operations
        let mu = ((self.maximal_delay_rounds.get() as f64
            * self.blending_ops_per_message as f64
            * self.normalization_constant.get())
            / self.membership_size.get() as f64)
            .ceil() as u64;
        (mu * self.minimum_messages_coefficient.get())
            ..=(mu * self.rounds_per_observation_window.get())
    }
}

impl IntervalStreamProvider for ObservationWindowTokioIntervalProvider {
    type IntervalStream =
        Box<dyn futures::Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let expected_message_range = self.calculate_expected_message_range();
        Box::new(
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
                Duration::from_secs(
                    (self.rounds_per_observation_window.get()) * self.round_duration_seconds.get(),
                ),
            ))
            .map(move |_| expected_message_range.clone()),
        )
    }
}
