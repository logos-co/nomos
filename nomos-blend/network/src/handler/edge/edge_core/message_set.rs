use core::task::{Context, Poll, Waker};

use libp2p::swarm::handler::FullyNegotiatedOutbound;

use crate::handler::edge::edge_core::{
    ready_to_send::{InternalState, ReadyToSendState},
    ConnectionEvent, ConnectionState, PollResult, StateTrait, LOG_TARGET,
};

pub struct MessageSetState {
    message: Vec<u8>,
    waker: Option<Waker>,
}

impl MessageSetState {
    pub const fn new(message: Vec<u8>) -> Self {
        Self {
            message,
            waker: None,
        }
    }
}

impl From<MessageSetState> for ConnectionState {
    fn from(value: MessageSetState) -> Self {
        Self::MessageSet(value)
    }
}

impl StateTrait for MessageSetState {
    fn on_connection_event(mut self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => {
                let updated_self = ReadyToSendState::new(
                    InternalState::MessageAndOutboundStreamSet(self.message, outbound_stream),
                );
                tracing::trace!(target: LOG_TARGET, "Transitioning from `MessageSet` to `ReadyToSend`.");
                if let Some(waker) = self.waker.take() {
                    waker.wake();
                }
                updated_self.into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                self.into()
            }
        }
    }

    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        self.waker = Some(cx.waker().clone());
        (Poll::Pending, self.into())
    }
}
