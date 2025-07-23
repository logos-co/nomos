#![allow(
    dead_code,
    reason = "At the moment this is only used in tests. This lint will go away once we integrate this connection handler."
)]

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use std::io;

use libp2p::{
    core::upgrade::{DeniedUpgrade, ReadyUpgrade},
    swarm::{ConnectionHandler, ConnectionHandlerEvent, SubstreamProtocol},
    StreamProtocol,
};

use crate::edge::handler::{dropped::DroppedState, sending::SendingState, starting::StartingState};

mod dropped;
mod sending;
mod starting;

const LOG_TARGET: &str = "blend::network::edge::handler";

type MessageSendFuture = Pin<Box<dyn Future<Output = Result<(), io::Error>> + Send>>;
#[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
type PollResult<T> = (
    Poll<
        ConnectionHandlerEvent<
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
            ToBehaviour,
        >,
    >,
    T,
);
#[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
type ConnectionEvent<'a> = libp2p::swarm::handler::ConnectionEvent<
    'a,
    <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
    <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
    <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
    <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
>;

enum ConnectionState {
    Starting(StartingState),
    Sending(SendingState),
    Dropped(DroppedState),
}

impl ConnectionState {
    fn on_connection_event(self, event: ConnectionEvent) -> Self {
        match self {
            Self::Starting(s) => s.on_connection_event(event),
            Self::Sending(s) => s.on_connection_event(event),
            Self::Dropped(s) => s.on_connection_event(event),
        }
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<Self> {
        match self {
            Self::Starting(s) => s.poll(cx),
            Self::Sending(s) => s.poll(cx),
            Self::Dropped(s) => s.poll(cx),
        }
    }
}

trait StateTrait: Into<ConnectionState> {
    fn on_connection_event(self, _event: ConnectionEvent) -> ConnectionState {
        self.into()
    }

    fn poll(self, cx: &mut Context<'_>) -> PollResult<ConnectionState>;
}

pub struct EdgeToCoreBlendConnectionHandler {
    state: Option<ConnectionState>,
}

impl EdgeToCoreBlendConnectionHandler {
    pub fn new(msg: Vec<u8>) -> Self {
        tracing::trace!(target: LOG_TARGET, "Initializing edge->core connection handler.");
        Self {
            state: Some(StartingState::new(msg).into()),
        }
    }
}

#[derive(Debug)]
pub struct SendError {
    pub reason: FailureReason,
    pub message: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureReason {
    UpgradeError,
    MessageStream,
}

#[derive(Debug)]
pub enum ToBehaviour {
    /// Notify the behaviour that the message was sent successfully.
    MessageSuccess(Vec<u8>),
    /// Notify the behaviour that the message could not be sent.
    SendError(SendError),
}

impl ConnectionHandler for EdgeToCoreBlendConnectionHandler {
    type FromBehaviour = ();
    type ToBehaviour = ToBehaviour;
    type InboundProtocol = DeniedUpgrade;
    type InboundOpenInfo = ();
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type OutboundOpenInfo = ();

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(DeniedUpgrade, ())
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
