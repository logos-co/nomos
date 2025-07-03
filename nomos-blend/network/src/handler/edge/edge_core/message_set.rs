use core::task::{Context, Poll, Waker};

use libp2p::swarm::handler::FullyNegotiatedOutbound;

use crate::handler::edge::edge_core::{
    ready_to_send::{InternalState, ReadyToSendState},
    ConnectionEvent, ConnectionState, PollResult, StateTrait, LOG_TARGET,
};

/// State indicating that a message has been passed to the connection handler by
/// the behavior but no outbound stream has yet been negotiated.
pub struct MessageSetState {
    /// The message to send when the stream is ready.
    message: Vec<u8>,
    /// The waker to wake when we need to force a new round of polling to
    /// progress the state machine.
    waker: Option<Waker>,
}

impl MessageSetState {
    pub const fn new(message: Vec<u8>, waker: Option<Waker>) -> Self {
        Self { message, waker }
    }
}

impl From<MessageSetState> for ConnectionState {
    fn from(value: MessageSetState) -> Self {
        Self::MessageSet(value)
    }
}

impl StateTrait for MessageSetState {
    // Moves the state machine to `ReadyToSendState` when the outbound stream is
    // negotiated.
    fn on_connection_event(mut self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => {
                tracing::trace!(target: LOG_TARGET, "Transitioning from `MessageSet` to `ReadyToSend`.");
                let updated_self = ReadyToSendState::new(
                    InternalState::MessageAndOutboundStreamSet(self.message, outbound_stream),
                    self.waker.take(),
                );
                updated_self.into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                // Nothing happened, no need to wake.
                self.into()
            }
        }
    }

    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        self.waker = Some(cx.waker().clone());
        // Once the state machine reaches this state, it moves to the next one only when
        // the stream is successfully negotiated.
        (Poll::Pending, self.into())
    }
}
