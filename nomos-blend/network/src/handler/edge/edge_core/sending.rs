use core::task::{Context, Poll};

use futures::FutureExt as _;
use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::edge_core::{
    dropped::DroppedState, ConnectionState, MessageSendFuture, PollResult, StateTrait, ToBehaviour,
    LOG_TARGET,
};

pub struct SendingState {
    pub message: Vec<u8>,
    pub outbound_message_send_future: MessageSendFuture,
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
                DroppedState {
                    error_message: Some("Failed to send message"),
                }
                .into(),
            )
        } else {
            (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::MessageSuccess(self.message),
                )),
                DroppedState {
                    error_message: None,
                }
                .into(),
            )
        }
    }
}
