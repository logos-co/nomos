use core::task::{Context, Poll, Waker};

use futures::TryFutureExt as _;
use libp2p::{core::upgrade::ReadyUpgrade, swarm::handler::OutboundUpgradeSend, StreamProtocol};

use crate::handler::{
    edge::edge_core::{
        sending::SendingState, ConnectionState, FromBehaviour, PollResult, StateTrait, LOG_TARGET,
    },
    send_msg,
};

/// State for when an outbound stream is negotiated, and a message is or is not
/// yet received from the behaviour.
pub struct ReadyToSendState {
    /// State indicating whether the message to send is present or not.
    state: InternalState,
    /// The waker to wake when we need to force a new round of polling to
    /// progress the state machine.
    waker: Option<Waker>,
}

impl ReadyToSendState {
    pub fn new(state: InternalState, waker: Option<Waker>) -> Self {
        // If we are being created with a full, ready state, we wake to start the
        // sending process.
        if let InternalState::MessageAndOutboundStreamSet(_, _) = state {
            if let Some(waker) = &waker {
                waker.wake_by_ref();
            }
        }
        Self { state, waker }
    }
}

impl From<ReadyToSendState> for ConnectionState {
    fn from(value: ReadyToSendState) -> Self {
        Self::ReadyToSend(value)
    }
}

pub enum InternalState {
    // Only the outbound stream has been negotiated.
    OnlyOutboundStreamSet(<ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output),
    // The outbound stream has been negotiated and there is a message to send to the other side of
    // the connection.
    MessageAndOutboundStreamSet(
        Vec<u8>,
        <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    ),
}

impl StateTrait for ReadyToSendState {
    // When the message is specified, the internal state is updated, ready to be
    // polled by the swarm to make progress.
    fn on_behaviour_event(mut self, event: FromBehaviour) -> ConnectionState {
        // Specifying a second message won't have any effect, since the connection
        // handler allows a single message to be sent to a core node per connection.
        let InternalState::OnlyOutboundStreamSet(inbound_stream) = self.state else {
            return self.into();
        };
        let FromBehaviour::Message(new_message) = event;

        let updated_self = Self {
            state: InternalState::MessageAndOutboundStreamSet(new_message, inbound_stream),
            waker: self.waker.take(),
        };
        tracing::trace!(target: LOG_TARGET, "Transitioning internal state from `OnlyOutboundStreamSet` to `MessageAndOutboundStreamSet`.");
        updated_self.into()
    }

    // If the state contains the message to be sent, it moves to the `Sending`
    // state, and starts the sending future.
    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        match self.state {
            InternalState::OnlyOutboundStreamSet(outbound_stream) => (
                // No message has been specified yet, we idle.
                Poll::Pending,
                Self {
                    state: InternalState::OnlyOutboundStreamSet(outbound_stream),
                    waker: Some(cx.waker().clone()),
                }
                .into(),
            ),
            InternalState::MessageAndOutboundStreamSet(message, outbound_stream) => {
                tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToSend` to `Sending`.");
                let sending_state = SendingState::new(
                    message.clone(),
                    Box::pin(send_msg(outbound_stream, message).map_ok(|_| ())),
                    Some(cx.waker().clone()),
                );
                (Poll::Pending, sending_state.into())
            }
        }
    }
}
