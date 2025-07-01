use core::task::{Context, Poll};

use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::edge_core::{ConnectionState, PollResult, StateTrait, ToBehaviour};

pub struct DroppedState {
    pub error_message: Option<&'static str>,
}

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
    fn poll(mut self, _cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Some(error_message) = self.error_message.take() else {
            return (Poll::Pending, self.into());
        };
        (
            Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                ToBehaviour::SendError(error_message),
            )),
            self.into(),
        )
    }
}
