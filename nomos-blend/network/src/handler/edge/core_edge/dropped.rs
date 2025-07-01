use core::task::{Context, Poll};

use libp2p::swarm::{ConnectionHandler, ConnectionHandlerEvent};

use crate::handler::{
    edge::core_edge::{ConnectionState, StateTrait, ToBehaviour},
    CoreToEdgeBlendConnectionHandler,
};

pub struct DroppedState;

impl From<DroppedState> for ConnectionState {
    fn from(value: DroppedState) -> Self {
        Self::Dropped(value)
    }
}

impl StateTrait for DroppedState {
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
