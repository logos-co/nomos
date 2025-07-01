use core::task::{Context, Poll, Waker};

use futures::TryFutureExt as _;
use libp2p::{core::upgrade::ReadyUpgrade, swarm::handler::OutboundUpgradeSend, StreamProtocol};

use crate::handler::{
    edge::edge_core::{
        sending::SendingState, ConnectionState, FromBehaviour, PollResult, StateTrait,
    },
    send_msg,
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
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        let InternalState::OnlyOutboundStreamSet(inbound_stream) = self.state else {
            return self.into();
        };
        let FromBehaviour::Message(new_message) = event;

        let updated_self = Self {
            state: InternalState::MessageAndOutboundStreamSet(new_message, inbound_stream),
            waker: None,
        };
        if let Some(waker) = self.waker {
            waker.wake();
        }
        updated_self.into()
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        match self.state {
            InternalState::OnlyOutboundStreamSet(outbound_stream) => (
                Poll::Pending,
                Self {
                    state: InternalState::OnlyOutboundStreamSet(outbound_stream),
                    waker: Some(cx.waker().clone()),
                }
                .into(),
            ),
            InternalState::MessageAndOutboundStreamSet(message, outbound_stream) => {
                let sending_state = SendingState {
                    message: message.clone(),
                    outbound_message_send_future: Box::pin(
                        send_msg(outbound_stream, message).map_ok(|_| ()),
                    ),
                };
                cx.waker().wake_by_ref();
                (Poll::Pending, sending_state.into())
            }
        }
    }
}
