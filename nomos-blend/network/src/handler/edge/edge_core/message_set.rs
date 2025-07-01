use std::task::{Context, Poll, Waker};

use libp2p::swarm::{
    handler::{ConnectionEvent, FullyNegotiatedOutbound},
    ConnectionHandler, ConnectionHandlerEvent,
};

use crate::handler::edge::{
    edge_core::{
        ready_to_send::{InternalState, ReadyToSendState},
        ConnectionState, StateTrait, ToBehaviour, LOG_TARGET,
    },
    EdgeToCoreBlendConnectionHandler,
};

pub struct MessageSetState {
    pub message: Vec<u8>,
    pub waker: Option<Waker>,
}

impl From<MessageSetState> for ConnectionState {
    fn from(value: MessageSetState) -> Self {
        Self::MessageSet(value)
    }
}

impl StateTrait for MessageSetState {
    fn on_connection_event(
        mut self,
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
            }) => {
                let updated_self = ReadyToSendState {
                    state: InternalState::MessageAndOutboundStreamSet(
                        self.message,
                        outbound_stream,
                    ),
                    waker: None,
                };
                if let Some(waker) = self.waker {
                    waker.wake();
                }
                updated_self.into()
            }
            event => {
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
