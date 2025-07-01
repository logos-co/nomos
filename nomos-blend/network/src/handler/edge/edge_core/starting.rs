use core::task::{Context, Poll};

use libp2p::swarm::handler::FullyNegotiatedOutbound;

use crate::handler::edge::edge_core::{
    dropped::DroppedState,
    message_set::MessageSetState,
    ready_to_send::{InternalState, ReadyToSendState},
    ConnectionEvent, ConnectionState, FromBehaviour, PollResult, StateTrait, LOG_TARGET,
};

pub struct StartingState;

impl From<StartingState> for ConnectionState {
    fn from(value: StartingState) -> Self {
        Self::Starting(value)
    }
}

impl StateTrait for StartingState {
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        let FromBehaviour::Message(new_message) = event;

        MessageSetState {
            message: new_message,
            waker: None,
        }
        .into()
    }

    fn on_connection_event(self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => ReadyToSendState {
                state: InternalState::OnlyOutboundStreamSet(outbound_stream),
                waker: None,
            }
            .into(),
            ConnectionEvent::DialUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
                DroppedState {
                    error_message: Some("Outbound upgrade error"),
                }
                .into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                self.into()
            }
        }
    }

    fn poll(self, _cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        (Poll::Pending, self.into())
    }
}
