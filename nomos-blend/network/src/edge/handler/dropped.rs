use core::task::{Context, Poll, Waker};

use libp2p::swarm::ConnectionHandlerEvent;

use crate::edge::handler::{ConnectionState, FailureReason, PollResult, StateTrait, ToBehaviour};

/// State indicating either that an error should be emitted, or that the state
/// machine has reached its end state, from which it does not exit anymore.
pub struct DroppedState {
    error: Option<FailureReason>,
    /// Used when this state is polled, to remember whether
    /// [`NotifyBehaviour`] has been requested to inform
    /// that the substream has been dropped.
    notify_behaviour_requested: bool,
}

impl DroppedState {
    pub fn new(error: Option<FailureReason>, waker: Option<Waker>) -> Self {
        // We wake here to notify the behaviour that the substream has been dropped.
        if let Some(waker) = waker {
            waker.wake();
        }
        Self {
            error,
            notify_behaviour_requested: false,
        }
    }
}

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
    /// Notifies the behaviour that the substream has been dropped
    /// by returning [`ConnectionHandlerEvent::NotifyBehaviour`]
    /// If it has been already notified, it returns `Poll::Pending`.
    fn poll(mut self, _cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        if self.notify_behaviour_requested {
            (Poll::Pending, self.into())
        } else {
            self.notify_behaviour_requested = true;
            (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::Dropped(self.error.take()),
                )),
                self.into(),
            )
        }
    }
}
