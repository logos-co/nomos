use core::task::{Context, Poll, Waker};

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{core::upgrade::ReadyUpgrade, swarm::handler::InboundUpgradeSend, StreamProtocol};

use crate::{
    core::handler::edge::{
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
        waker: Option<Waker>,
    ) -> Self {
        // We wake here because we want the timeout to be polled in order to be
        // started.
        if let Some(waker) = waker {
            waker.wake();
        }
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
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout), Some(cx.waker().clone())).into(),
            );
        };
        tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToReceive` to `Receiving`.");
        let receiving_state = ReceivingState::new(
            self.timeout_timer,
            Box::pin(recv_msg(self.inbound_stream).map_ok(|(_, message)| message)),
            Some(cx.waker().clone()),
        );
        (Poll::Pending, receiving_state.into())
    }
}
