use std::task::{Context, Poll};

use libp2p::swarm::{ConnectionHandler, ConnectionHandlerEvent};

use crate::handler::edge::{
    edge_core::{ConnectionState, MessageSendFuture, StateTrait, ToBehaviour},
    EdgeToCoreBlendConnectionHandler,
};

pub struct SendingState {
    pub message: Vec<u8>,
    pub outbound_message_send_future: MessageSendFuture,
}

impl From<SendingState> for ConnectionState {
    fn from(value: SendingState) -> Self {
        Self::Sending(value)
    }
}

impl StateTrait for SendingState {
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
