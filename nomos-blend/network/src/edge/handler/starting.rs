use core::task::{Context, Poll, Waker};

use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::FullyNegotiatedOutbound, ConnectionHandlerEvent, SubstreamProtocol},
};

use crate::{
    edge::handler::{
        dropped::DroppedState,
        message_set::MessageSetState,
        ready_to_send::{InternalState, ReadyToSendState},
        ConnectionEvent, ConnectionState, FailureReason, FromBehaviour, PollResult, StateTrait,
        LOG_TARGET,
    },
    PROTOCOL_NAME,
};

/// Entrypoint to start sending a single message to a core node.
pub struct StartingState {
    /// Used when this state is polled, to remember whether a new outbound
    /// stream request has already been made to the swarm.
    connection_requested: bool,
    /// The waker to wake when we need to force a new round of polling to
    /// progress the state machine.
    waker: Option<Waker>,
}

impl StartingState {
    pub const fn new() -> Self {
        Self {
            connection_requested: false,
            waker: None,
        }
    }
}

impl From<StartingState> for ConnectionState {
    fn from(value: StartingState) -> Self {
        Self::Starting(value)
    }
}

impl StateTrait for StartingState {
    // Moves the state machine to the `MessageSetState` state, with the provided
    // message.
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        let FromBehaviour::Message(new_message) = event;

        MessageSetState::new(new_message, self.waker).into()
    }

    // When an inbound substream is negotiated, it moves the state machine to a
    // `ReadyToSendState` state with an `OnlyOutboundStreamSet` internal state,
    // since no message has been passed to the connection handler by the behavior
    // yet. In case of `DialUpgradeError`, the state machine is moved to the
    // `DroppedState` with the relative error.
    fn on_connection_event(mut self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => ReadyToSendState::new(
                InternalState::OnlyOutboundStreamSet(outbound_stream),
                self.waker.take(),
            )
            .into(),
            ConnectionEvent::DialUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
                DroppedState::new(Some(FailureReason::UpgradeError), self.waker.take()).into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                // Nothing happened, no need to wake.
                self.into()
            }
        }
    }

    // When polled, if a connection has not been requested, it emits a
    // `OutboundSubstreamRequest` request, and waits until the swarm propagates the
    // necessary events.
    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        if self.connection_requested {
            (Poll::Pending, self.into())
        } else {
            tracing::trace!(target: LOG_TARGET, "Requesting a new outbound substream.");
            // Progress will be made with the generated event. No need to wake here.
            (
                Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                    protocol: SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ()),
                }),
                Self {
                    connection_requested: true,
                    waker: Some(cx.waker().clone()),
                }
                .into(),
            )
        }
    }
}
