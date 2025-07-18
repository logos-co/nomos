use core::task::{Context, Poll, Waker};

use futures::TryFutureExt as _;
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::OutboundUpgradeSend, ConnectionHandlerEvent},
    StreamProtocol,
};

use super::ToBehaviour;
use crate::{
    edge::handler::{
        dropped::DroppedState, sending::SendingState, ConnectionState, FromBehaviour, PollResult,
        StateTrait, LOG_TARGET,
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
        if let InternalState::MessageAndOutboundStreamSet { .. } = state {
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
    OnlyOutboundStreamSet {
        stream: <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
        /// Used when this state is polled, to remember whether
        /// [`NotifyBehaviour`] has been requested to inform that it's
        /// ready to send a message.
        notify_behaviour_requested: bool,
    },
    // The outbound stream has been negotiated and there is a message to send to the other side of
    // the connection.
    MessageAndOutboundStreamSet {
        message: Vec<u8>,
        stream: <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    },
}

impl InternalState {
    pub(crate) const fn only_outbound_stream_set(
        stream: <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    ) -> Self {
        Self::OnlyOutboundStreamSet {
            stream,
            notify_behaviour_requested: false,
        }
    }

    pub(crate) const fn message_and_outbound_stream_set(
        message: Vec<u8>,
        stream: <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    ) -> Self {
        Self::MessageAndOutboundStreamSet { message, stream }
    }
}

impl StateTrait for ReadyToSendState {
    fn on_behaviour_event(mut self, event: FromBehaviour) -> ConnectionState {
        match event {
            // When the message is specified, the internal state is updated, ready to be
            // polled by the swarm to make progress.
            FromBehaviour::Message(message) => {
                // Specifying a second message won't have any effect, since the connection
                // handler allows a single message to be sent to a core node per connection.
                let InternalState::OnlyOutboundStreamSet { stream, .. } = self.state else {
                    return self.into();
                };
                let updated_self = Self {
                    state: InternalState::MessageAndOutboundStreamSet { message, stream },
                    waker: self.waker.take(),
                };
                tracing::trace!(target: LOG_TARGET, "Transitioning internal state from `OnlyOutboundStreamSet` to `MessageAndOutboundStreamSet`.");
                updated_self.into()
            }
            FromBehaviour::DropSubstream => {
                tracing::trace!(target: LOG_TARGET, "ReadyToSendState -> DroppedState by request from behaviour.");
                DroppedState::new(None, self.waker.take()).into()
            }
        }
    }

    // If the state contains the message to be sent, it moves to the `Sending`
    // state, and starts the sending future.
    // Otherwise, notify the behaviour only once that it is ready to send a message.
    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        match self.state {
            InternalState::OnlyOutboundStreamSet {
                stream,
                notify_behaviour_requested,
            } => {
                if notify_behaviour_requested {
                    // No message has been specified yet, we idle.
                    (
                        Poll::Pending,
                        Self::new(
                            InternalState::OnlyOutboundStreamSet {
                                stream,
                                notify_behaviour_requested,
                            },
                            Some(cx.waker().clone()),
                        )
                        .into(),
                    )
                } else {
                    // Notify the behaviour that the connection handler is ready to send a message.
                    (
                        Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            ToBehaviour::ReadyToSend,
                        )),
                        Self::new(
                            InternalState::OnlyOutboundStreamSet {
                                stream,
                                notify_behaviour_requested: true,
                            },
                            Some(cx.waker().clone()),
                        )
                        .into(),
                    )
                }
            }
            InternalState::MessageAndOutboundStreamSet { message, stream } => {
                tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToSend` to `Sending`.");
                let sending_state = SendingState::new(
                    message.clone(),
                    Box::pin(send_msg(stream, message).map_ok(|_| ())),
                    Some(cx.waker().clone()),
                );
                (Poll::Pending, sending_state.into())
            }
        }
    }
}
