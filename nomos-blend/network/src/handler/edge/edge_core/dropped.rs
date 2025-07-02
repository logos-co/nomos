use core::task::{Context, Poll};

use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::edge_core::{
    ConnectionState, FailureReason, PollResult, StateTrait, ToBehaviour,
};

/// State indicating either that an error should be emitted, or that the state
/// machine has reached its end state, from which it does not exit anymore.
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
    // If an error message is to be emitted, it returns `Poll::Ready`, consuming it.
    // After an error is consumed or if no error is to be consumed, the state
    // machine will indefinitely return `Poll::Pending` every time it is polled.
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
