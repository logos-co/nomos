use core::task::{Context, Poll};

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{core::upgrade::ReadyUpgrade, swarm::handler::InboundUpgradeSend, StreamProtocol};

use crate::handler::{
    edge::core_edge::{
        dropped::DroppedState, receiving::ReceivingState, ConnectionState, FailureReason,
        PollResult, StateTrait, TimerFuture, LOG_TARGET,
    },
    recv_msg,
};

/// State reached after a successful inbound stream negotiation.
pub struct ReadyToReceiveState {
    /// The inbound stream as returned by libp2p after successful negotiation.
    inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
    /// The timer future that will be polled regularly to close the connection
    /// when idling for too long.
    timeout_timer: TimerFuture,
}

impl ReadyToReceiveState {
    pub fn new(
        timeout_timer: TimerFuture,
        inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
    ) -> Self {
        Self {
            inbound_stream,
            timeout_timer,
        }
    }
}

impl From<ReadyToReceiveState> for ConnectionState {
    fn from(value: ReadyToReceiveState) -> Self {
        Self::ReadyToReceive(value)
    }
}

impl StateTrait for ReadyToReceiveState {
    // If the timer elapses, moves the state machine to `DroppedState` with a
    // timeout error. Otherwise, it moves it to `ReceivingState` with the `recv_msg`
    // future to fetch the first message from the stream.
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without starting the reception of the message. Closing the connection.");
            // We wake here because we want the new error to be consumed.
            cx.waker().wake_by_ref();
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout)).into(),
            );
        };
        let receiving_state = ReceivingState::new(
            self.timeout_timer,
            Box::pin(recv_msg(self.inbound_stream).map_ok(|(_, message)| message)),
        );
        tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToReceive` to `Receiving`.");
        // We wake here since the future to receive the message must be polled once to
        // start making any progress.
        cx.waker().wake_by_ref();
        (Poll::Pending, receiving_state.into())
    }
}
