use std::task::{Context, Poll};

use libp2p::swarm::{ConnectionHandler, ConnectionHandlerEvent};

use crate::handler::edge::{
    edge_core::{ConnectionState, StateTrait, ToBehaviour},
    EdgeToCoreBlendConnectionHandler,
};

pub struct DroppedState {
    pub error_message: &'static str,
}

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
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
