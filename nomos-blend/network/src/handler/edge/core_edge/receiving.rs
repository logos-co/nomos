use core::task::{Context, Poll};

use futures::FutureExt as _;
use libp2p::swarm::ConnectionHandlerEvent;

use crate::handler::edge::core_edge::{
    dropped::DroppedState, ConnectionState, FailureReason, MessageReceiveFuture, PollResult,
    StateTrait, TimerFuture, ToBehaviour, LOG_TARGET,
};

/// State representing the moment in which a new message is being received from
/// a peer.
pub struct ReceivingState {
    /// The future that will resolve when the attempt to receive the message is
    /// completed, either successfully or with an error.
    incoming_message: MessageReceiveFuture,
    /// The timer future that will be polled regularly to close the connection
    /// if receiving the message takes too long.
    timeout_timer: TimerFuture,
}

impl ReceivingState {
    pub fn new(timeout_timer: TimerFuture, incoming_message: MessageReceiveFuture) -> Self {
        Self {
            incoming_message,
            timeout_timer,
        }
    }
}

impl From<ReceivingState> for ConnectionState {
    fn from(value: ReceivingState) -> Self {
        Self::Receiving(value)
    }
}

impl StateTrait for ReceivingState {
    // If the timer elapses, moves the state machine to `DroppedState` with a
    // timeout error. Otherwise, if the message is correctly received it generates a
    // new message, else it moves to the `DroppedState` with the message reception
    // error.
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without completing the reception of the message. Closing the connection.");
            // We wake here because we want the new error to be consumed.
            cx.waker().wake_by_ref();
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout)).into(),
            );
        };
        let Poll::Ready(message_receive_result) = self.incoming_message.poll_unpin(cx) else {
            // We don't wake here since the `incoming_message` future will wake the waker
            // when completed.
            return (Poll::Pending, self.into());
        };
        match message_receive_result {
            Err(error) => {
                tracing::error!(target: LOG_TARGET, "Failed to receive message. Error {error:?}");
                // We wake here because we want the new error to be consumed.
                cx.waker().wake_by_ref();
                (
                    Poll::Pending,
                    DroppedState::new(Some(FailureReason::MessageStream)).into(),
                )
            }
            Ok(message) => {
                // We don't wake here since we return `Poll::Ready`.
                tracing::trace!(target: LOG_TARGET, "Message received successfully. Transitioning from `Receiving` to `Dropped`.");
                (
                    Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::Message(message),
                    )),
                    DroppedState::new(None).into(),
                )
            }
        }
    }
}
