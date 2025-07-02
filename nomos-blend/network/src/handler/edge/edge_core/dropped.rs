use core::task::{Context, Poll, Waker};

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
    pub fn new(error: Option<FailureReason>, waker: Option<Waker>) -> Self {
        let Some(error_to_consume) = error else {
            // No need to wake if we don't have an error to report.
            return Self { error: None };
        };
        // We wake here because we want the new error to be consumed.
        if let Some(waker) = waker {
            waker.wake();
        }
        Self {
            error: Some(error_to_consume),
        }
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
