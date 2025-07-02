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
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        // Specifying a second message won't have any effect, since the connection
        // handler allows a single message to be sent to a core node per connection.
        let InternalState::OnlyOutboundStreamSet(inbound_stream) = self.state else {
            return self.into();
        };
        let FromBehaviour::Message(new_message) = event;

        let updated_self = Self {
            state: InternalState::MessageAndOutboundStreamSet(new_message, inbound_stream),
            waker: None,
        };
        tracing::trace!(target: LOG_TARGET, "Transitioning internal state from `OnlyOutboundStreamSet` to `MessageAndOutboundStreamSet`.");
        // We wake here because we want this state to be polled again to make progress.
        if let Some(waker) = self.waker {
            waker.wake();
        }
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
                let sending_state = SendingState::new(
                    message.clone(),
                    Box::pin(send_msg(outbound_stream, message).map_ok(|_| ())),
                );
                tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToSend` to `Sending`.");
                // We wake for the sending future to be polled and started.
                cx.waker().wake_by_ref();
                (Poll::Pending, sending_state.into())
            }
        }
    }
}
