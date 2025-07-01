use std::task::{Context, Poll};

use libp2p::swarm::{
    handler::{ConnectionEvent, FullyNegotiatedOutbound},
    ConnectionHandler, ConnectionHandlerEvent,
};

use crate::handler::edge::{
    edge_core::{
        dropped::DroppedState,
        message_set::MessageSetState,
        ready_to_send::{InternalState, ReadyToSendState},
        ConnectionState, FromBehaviour, StateTrait, ToBehaviour, LOG_TARGET,
    },
    EdgeToCoreBlendConnectionHandler,
};

pub struct StartingState;

impl From<StartingState> for ConnectionState {
    fn from(value: StartingState) -> Self {
        Self::Starting(value)
    }
}

impl StateTrait for StartingState {
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        let FromBehaviour::Message(new_message) = event;

        MessageSetState {
            message: new_message,
            waker: None,
        }
        .into()
    }

    fn on_connection_event(
        self,
        event: ConnectionEvent<
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
        >,
    ) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => ReadyToSendState {
                state: InternalState::OnlyOutboundStreamSet(outbound_stream),
                waker: None,
            }
            .into(),
            ConnectionEvent::DialUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
                DroppedState {
                    error_message: "Outbound upgrade error",
                }
                .into()
            }
            _ => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {event:?}");
                self.into()
            }
        }
    }

    fn poll(
        self,
        cx: &mut Context<'_>,
    ) -> (
        Poll<
            ConnectionHandlerEvent<
                <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
                <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
                ToBehaviour,
            >,
        >,
        ConnectionState,
    ) {
        unimplemented!()
    }
}
