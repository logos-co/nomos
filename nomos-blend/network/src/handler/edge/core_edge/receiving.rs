use core::task::{Context, Poll};

use futures::FutureExt as _;
use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::core_edge::{
    dropped::DroppedState, ConnectionState, FailureReason, MessageReceiveFuture, PollResult,
    StateTrait, TimerFuture, ToBehaviour, LOG_TARGET,
};

pub struct ReceivingState {
    pub timeout_timer: TimerFuture,
    pub incoming_message: MessageReceiveFuture,
}

impl From<ReceivingState> for ConnectionState {
    fn from(value: ReceivingState) -> Self {
        Self::Receiving(value)
    }
}

impl StateTrait for ReceivingState {
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without completing the reception of the message. Closing the connection.");
            cx.waker().wake_by_ref();
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout)).into(),
            );
        };
        let Poll::Ready(message_receive_result) = self.incoming_message.poll_unpin(cx) else {
            return (Poll::Pending, self.into());
        };
        match message_receive_result {
            Err(error) => {
                tracing::error!(target: LOG_TARGET, "Failed to receive message. Error {error:?}");
                cx.waker().wake_by_ref();
                (
                    Poll::Pending,
                    DroppedState::new(Some(FailureReason::MessageStream)).into(),
                )
            }
            Ok(message) => (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::Message(message),
                )),
                DroppedState::new(None).into(),
            ),
        }
    }
}
