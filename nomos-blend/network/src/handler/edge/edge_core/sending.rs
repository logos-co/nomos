use core::task::{Context, Poll};

use futures::FutureExt as _;
use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::edge_core::{
    dropped::DroppedState, ConnectionState, FailureReason, MessageSendFuture, PollResult,
    StateTrait, ToBehaviour, LOG_TARGET,
};

pub struct SendingState {
    message: Vec<u8>,
    outbound_message_send_future: MessageSendFuture,
}

impl SendingState {
    pub fn new(message: Vec<u8>, outbound_message_send_future: MessageSendFuture) -> Self {
        Self {
            message,
            outbound_message_send_future,
        }
    }
}

impl From<SendingState> for ConnectionState {
    fn from(value: SendingState) -> Self {
        Self::Sending(value)
    }
}

impl StateTrait for SendingState {
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Ready(message_send_result) = self.outbound_message_send_future.poll_unpin(cx)
        else {
            return (Poll::Pending, self.into());
        };
        if let Err(error) = message_send_result {
            tracing::error!(target: LOG_TARGET, "Failed to send message. Error {error:?}");
            cx.waker().wake_by_ref();
            (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::MessageStream)).into(),
            )
        } else {
            tracing::trace!(target: LOG_TARGET, "Transitioning from `Sending` to `Dropped`.");
            (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::MessageSuccess(self.message),
                )),
                DroppedState::new(None).into(),
            )
        }
    }
}
