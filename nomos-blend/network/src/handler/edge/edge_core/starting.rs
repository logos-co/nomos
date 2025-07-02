use core::task::{Context, Poll, Waker};

use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::FullyNegotiatedOutbound, ConnectionHandlerEvent, SubstreamProtocol},
};

use crate::handler::{
    edge::edge_core::{
        dropped::DroppedState,
        message_set::MessageSetState,
        ready_to_send::{InternalState, ReadyToSendState},
        ConnectionEvent, ConnectionState, FailureReason, FromBehaviour, PollResult, StateTrait,
        LOG_TARGET,
    },
    PROTOCOL_NAME,
};

pub struct StartingState {
    connection_requested: bool,
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
    fn on_behaviour_event(self, event: FromBehaviour) -> ConnectionState {
        let FromBehaviour::Message(new_message) = event;

        MessageSetState::new(new_message).into()
    }

    fn on_connection_event(mut self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outbound_stream,
                ..
            }) => {
                ReadyToSendState::new(InternalState::OnlyOutboundStreamSet(outbound_stream)).into()
            }
            ConnectionEvent::DialUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
                if let Some(waker) = self.waker.take() {
                    waker.wake();
                }
                DroppedState::new(Some(FailureReason::UpgradeError)).into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                self.into()
            }
        }
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        if self.connection_requested {
            (Poll::Pending, self.into())
        } else {
            tracing::trace!(target: LOG_TARGET, "Requesting a new outbound substream.");
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
