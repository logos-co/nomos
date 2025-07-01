use std::task::{Context, Poll, Waker};

use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::OutboundUpgradeSend, ConnectionHandler, ConnectionHandlerEvent},
    StreamProtocol,
};

use crate::handler::edge::{
    edge_core::{ConnectionState, FromBehaviour, StateTrait, ToBehaviour},
    EdgeToCoreBlendConnectionHandler,
};

pub struct ReadyToSendState {
    pub state: InternalState,
    pub waker: Option<Waker>,
}

impl From<ReadyToSendState> for ConnectionState {
    fn from(value: ReadyToSendState) -> Self {
        Self::ReadyToSend(value)
    }
}

pub enum InternalState {
    OnlyOutboundStreamSet(<ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output),
    MessageAndOutboundStreamSet(
        Vec<u8>,
        <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    ),
}

impl StateTrait for ReadyToSendState {
    fn on_behaviour_event(mut self, event: FromBehaviour) -> ConnectionState {
        let InternalState::OnlyOutboundStreamSet(inbound_stream) = self.state else {
            return self.into();
        };
        let FromBehaviour::Message(new_message) = event;
        let waker = self.waker.take();

        let updated_self = Self {
            state: InternalState::MessageAndOutboundStreamSet(new_message, inbound_stream),
            waker: None,
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        updated_self.into()
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
