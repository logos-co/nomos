use core::task::{Context, Poll, Waker};

use futures::TryFutureExt as _;
use libp2p::{core::upgrade::ReadyUpgrade, swarm::handler::OutboundUpgradeSend, StreamProtocol};

use crate::handler::{
    edge::edge_core::{
        sending::SendingState, ConnectionState, FromBehaviour, PollResult, StateTrait, LOG_TARGET,
    },
    send_msg,
};

pub struct ReadyToSendState {
    state: InternalState,
    waker: Option<Waker>,
}

impl ReadyToSendState {
    pub const fn new(state: InternalState) -> Self {
        Self { state, waker: None }
    }
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
        tracing::trace!(target: LOG_TARGET, "Transitioning internal state from `OnlyOutboundStreamSet` to `MessageAndOutboundStreamSet`.");
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
                let sending_state = SendingState::new(
                    message.clone(),
                    Box::pin(send_msg(outbound_stream, message).map_ok(|_| ())),
                );
                tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToSend` to `Sending`.");
                cx.waker().wake_by_ref();
                (Poll::Pending, sending_state.into())
            }
        }
    }
}
