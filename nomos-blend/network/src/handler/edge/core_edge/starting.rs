use core::{
    task::{Context, Poll},
    time::Duration,
};

use libp2p::swarm::{
    handler::{ConnectionEvent, FullyNegotiatedInbound},
    ConnectionHandler, ConnectionHandlerEvent,
};
use tokio::time::sleep;

use crate::handler::{
    edge::core_edge::{
        dropped::DroppedState, ready_to_receive::ReadyToReceiveState, ConnectionState, StateTrait,
        ToBehaviour, LOG_TARGET,
    },
    CoreToEdgeBlendConnectionHandler,
};

pub struct StartingState {
    pub connection_timeout: Duration,
}

impl From<StartingState> for ConnectionState {
    fn from(value: StartingState) -> Self {
        Self::Starting(value)
    }
}

impl StateTrait for StartingState {
    fn on_connection_event(
        self,
        event: ConnectionEvent<
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
        >,
    ) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: inbound_stream,
                ..
            }) => ReadyToReceiveState {
                timeout_timer: Box::pin(sleep(self.connection_timeout)),
                inbound_stream,
            }
            .into(),
            ConnectionEvent::ListenUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Inbound upgrade error: {error:?}");
                DroppedState.into()
            }
            _ => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {event:?}");
                self.into()
            }
        }
    }

    fn poll(
        self,
        _cx: &mut Context<'_>,
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
        (Poll::Pending, self.into())
    }
}
