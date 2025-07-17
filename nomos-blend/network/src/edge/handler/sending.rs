use core::task::{Context, Poll, Waker};

use futures::FutureExt as _;
use libp2p::swarm::ConnectionHandlerEvent;

use crate::edge::handler::{
    dropped::DroppedState, ConnectionState, FailureReason, MessageSendFuture, PollResult,
    StateTrait, ToBehaviour, LOG_TARGET,
};

/// State representing the moment in which a new message is being sent to the
/// peer.
pub struct SendingState {
    /// The message being sent.
    message: Vec<u8>,
    /// The sending future.
    outbound_message_send_future: MessageSendFuture,
}

impl SendingState {
    pub fn new(
        message: Vec<u8>,
        outbound_message_send_future: MessageSendFuture,
        waker: Option<Waker>,
    ) -> Self {
        // We wake because we want to poll the message sending future to make progress.
        if let Some(waker) = waker {
            waker.wake();
        }
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
    /// If the sending future completes, we return either the success event to
    /// the behaviour, or move to the `Dropped` state with the generated error.
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Ready(message_send_result) = self.outbound_message_send_future.poll_unpin(cx)
        else {
            // No wake since the future will wake the waker.
            return (Poll::Pending, self.into());
        };
        if let Err(error) = message_send_result {
            tracing::error!(target: LOG_TARGET, "Failed to send message. Error {error:?}");
            (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::MessageStream), Some(cx.waker().clone()))
                    .into(),
            )
        } else {
            tracing::trace!(target: LOG_TARGET, "Message sent successfully. Transitioning from `Sending` to `Dropped`.");
            (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::MessageSuccess(self.message),
                )),
                DroppedState::new(None, Some(cx.waker().clone())).into(),
            )
        }
    }
}
