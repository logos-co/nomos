use core::task::{Context, Poll};

use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::edge_core::{
    ConnectionState, FailureReason, PollResult, StateTrait, ToBehaviour,
};

pub struct DroppedState {
    error: Option<FailureReason>,
}

impl DroppedState {
    pub const fn new(error: Option<FailureReason>) -> Self {
        Self { error }
    }
}

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
    fn poll(mut self, _cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let poll_result = self.error.take().map_or_else(
            || Poll::Pending,
            |error| {
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::SendError(error),
                ))
            },
        );
        (poll_result, self.into())
    }
}
