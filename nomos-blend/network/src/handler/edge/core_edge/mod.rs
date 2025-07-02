use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use std::io;

use libp2p::{
    core::upgrade::{DeniedUpgrade, ReadyUpgrade},
    swarm::{ConnectionHandler, ConnectionHandlerEvent, SubstreamProtocol},
    StreamProtocol,
};

use crate::handler::{
    edge::core_edge::{
        dropped::DroppedState, ready_to_receive::ReadyToReceiveState, receiving::ReceivingState,
        starting::StartingState,
    },
    PROTOCOL_NAME,
};

mod dropped;
mod ready_to_receive;
mod receiving;
mod starting;

const LOG_TARGET: &str = "blend::libp2p::handler::core-edge";

type TimerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type MessageReceiveFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, io::Error>> + Send>>;
#[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
type PollResult<T> = (
    Poll<
        ConnectionHandlerEvent<
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
            ToBehaviour,
        >,
    >,
    T,
);
#[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
type ConnectionEvent<'a> = libp2p::swarm::handler::ConnectionEvent<
    'a,
    <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
    <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
    <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
    <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
>;

pub enum ConnectionState {
    Starting(StartingState),
    ReadyToReceive(ReadyToReceiveState),
    Receiving(ReceivingState),
    Dropped(DroppedState),
}

impl ConnectionState {
    fn on_connection_event(self, event: ConnectionEvent) -> Self {
        match self {
            Self::Starting(s) => s.on_connection_event(event),
            Self::ReadyToReceive(s) => s.on_connection_event(event),
            Self::Receiving(s) => s.on_connection_event(event),
            Self::Dropped(s) => s.on_connection_event(event),
        }
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<Self> {
        match self {
            Self::Starting(s) => s.poll(cx),
            Self::ReadyToReceive(s) => s.poll(cx),
            Self::Receiving(s) => s.poll(cx),
            Self::Dropped(s) => s.poll(cx),
        }
    }
}

trait StateTrait: Into<ConnectionState> {
    fn on_connection_event(self, event: ConnectionEvent) -> ConnectionState {
        if let ConnectionEvent::ListenUpgradeError(error) = event {
            tracing::trace!(target: LOG_TARGET, "Inbound upgrade error: {error:?}");
            return DroppedState.into();
        }
        self.into()
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState>;
}

pub struct CoreToEdgeBlendConnectionHandler {
    state: Option<ConnectionState>,
}

impl CoreToEdgeBlendConnectionHandler {
    pub fn new(connection_timeout: Duration) -> Self {
        Self {
            state: Some(StartingState { connection_timeout }.into()),
        }
    }
}

#[derive(Debug)]
pub enum ToBehaviour {
    /// A message has been received from the connection.
    Message(Vec<u8>),
    FailedReception,
}

impl ConnectionHandler for CoreToEdgeBlendConnectionHandler {
    type FromBehaviour = ();
    type ToBehaviour = ToBehaviour;
    type InboundProtocol = ReadyUpgrade<StreamProtocol>;
    type InboundOpenInfo = ();
    type OutboundProtocol = DeniedUpgrade;
    type OutboundOpenInfo = ();

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ())
    }

    fn on_behaviour_event(&mut self, _event: Self::FromBehaviour) {}

    fn on_connection_event(&mut self, event: ConnectionEvent) {
        let state = self.state.take().expect("Inconsistent state");
        self.state = Some(state.on_connection_event(event));
    }

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        let state = self.state.take().expect("Inconsistent state");

        let (poll_result, new_state) = state.poll(cx);
        self.state = Some(new_state);

        poll_result
    }
}
