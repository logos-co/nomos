use core::task::{Context, Poll, Waker};

use libp2p::swarm::handler::FullyNegotiatedOutbound;

use crate::handler::edge::edge_core::{
    ready_to_send::{InternalState, ReadyToSendState},
    ConnectionEvent, ConnectionState, PollResult, StateTrait, LOG_TARGET,
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
    fn on_connection_event(self, event: ConnectionEvent) -> ConnectionState {
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
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                self.into()
            }
        }
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        (
            Poll::Pending,
            Self {
                message: self.message,
                waker: Some(cx.waker().clone()),
            }
            .into(),
        )
    }
}
