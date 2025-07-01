use core::task::{Context, Poll};

use crate::handler::edge::core_edge::{ConnectionState, PollResult, StateTrait};

pub struct DroppedState;

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
    fn poll(self, _cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        (Poll::Pending, self.into())
    }
}
