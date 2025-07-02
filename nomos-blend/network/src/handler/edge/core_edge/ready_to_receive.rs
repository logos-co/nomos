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

pub struct ReadyToReceiveState {
    inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
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
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without starting the reception of the message. Closing the connection.");
            cx.waker().wake_by_ref();
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout)).into(),
            );
        };
        let receiving_state = ReceivingState {
            incoming_message: Box::pin(
                recv_msg(self.inbound_stream).map_ok(|(_, message)| message),
            ),
            timeout_timer: self.timeout_timer,
        };
        tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToReceive` to `Receiving`.");
        // We awake here since the future to receive the message must be polled once to
        // start making any progress.
        cx.waker().wake_by_ref();
        (Poll::Pending, receiving_state.into())
    }
}
