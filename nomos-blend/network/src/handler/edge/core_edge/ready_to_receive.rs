use core::task::{Context, Poll};

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::InboundUpgradeSend, ConnectionHandler, ConnectionHandlerEvent},
    StreamProtocol,
};

use crate::handler::{
    edge::core_edge::{
        dropped::DroppedState, receiving::ReceivingState, ConnectionState, StateTrait, TimerFuture,
        ToBehaviour, LOG_TARGET,
    },
    recv_msg, CoreToEdgeBlendConnectionHandler,
};

pub struct ReadyToReceiveState {
    pub timeout_timer: TimerFuture,
    pub inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
}

impl From<ReadyToReceiveState> for ConnectionState {
    fn from(value: ReadyToReceiveState) -> Self {
        Self::ReadyToReceive(value)
    }
}

impl StateTrait for ReadyToReceiveState {
    fn poll(
        mut self,
        cx: &mut Context<'_>,
    ) -> (
        Poll<
            ConnectionHandlerEvent<
                <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
                <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
                ToBehaviour,
            >,
        >,
        ConnectionState,
    ) {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without starting the reception of the message. Closing the connection.");
            return (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::FailedReception,
                )),
                DroppedState.into(),
            );
        };
        (
            Poll::Pending,
            ReceivingState {
                incoming_message: Box::pin(
                    recv_msg(self.inbound_stream).map_ok(|(_, message)| message),
                ),
                timeout_timer: self.timeout_timer,
            }
            .into(),
        )
    }
}
