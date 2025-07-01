use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use std::io;

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{
    core::upgrade::{DeniedUpgrade, ReadyUpgrade},
    swarm::{
        handler::{ConnectionEvent, FullyNegotiatedOutbound, OutboundUpgradeSend},
        ConnectionHandler, ConnectionHandlerEvent, SubstreamProtocol,
    },
    StreamProtocol,
};

use crate::handler::{
    edge::edge_core::{
        dropped::DroppedState, message_set::MessageSetState, ready_to_send::ReadyToSendState,
        sending::SendingState, starting::StartingState,
    },
    send_msg,
};

mod dropped;
mod message_set;
mod ready_to_send;
mod sending;
mod starting;

const LOG_TARGET: &str = "blend::libp2p::handler::edge-core";

type MessageSendFuture = Pin<Box<dyn Future<Output = Result<(), io::Error>> + Send>>;

enum ConnectionState {
    Starting(StartingState),
    MessageSet(MessageSetState),
    ReadyToSend(ReadyToSendState),
    Sending(SendingState),
    Dropped(DroppedState),
}

impl ConnectionState {
    fn on_behaviour_event(self, event: FromBehaviour) -> Self {
        match self {
            Self::Starting(s) => s.on_behaviour_event(event),
            Self::MessageSet(s) => s.on_behaviour_event(event),
            Self::ReadyToSend(s) => s.on_behaviour_event(event),
            Self::Sending(s) => s.on_behaviour_event(event),
            Self::Dropped(s) => s.on_behaviour_event(event),
        }
    }

    fn on_connection_event(
        self,
        event: ConnectionEvent<
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
        >,
    ) -> Self {
        match self {
            Self::Starting(s) => s.on_connection_event(event),
            Self::MessageSet(s) => s.on_connection_event(event),
            Self::ReadyToSend(s) => s.on_connection_event(event),
            Self::Sending(s) => s.on_connection_event(event),
            Self::Dropped(s) => s.on_connection_event(event),
        }
    }

    fn poll(
        self,
        cx: &mut Context<'_>,
    ) -> (
        Poll<
            ConnectionHandlerEvent<
                <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
                <CoreToEdgeBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
                ToBehaviour,
            >,
        >,
        ConnectionState,
    ) {
        match self {
            Self::Starting(s) => s.poll(cx),
            Self::MessageSet(s) => s.poll(cx),
            Self::ReadyToSend(s) => s.poll(cx),
            Self::Sending(s) => s.poll(cx),
            Self::Dropped(s) => s.poll(cx),
        }
    }
}

trait StateTrait: Into<ConnectionState> {
    fn on_behaviour_event(self, _event: FromBehaviour) -> ConnectionState {
        self.into()
    }

    fn on_connection_event(
        self,
        event: ConnectionEvent<
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::InboundOpenInfo,
            <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
        >,
    ) -> ConnectionState {
        if let ConnectionEvent::DialUpgradeError(error) = event {
            tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
            return DroppedState { error_message: "Outbound upgrade error." } .into();
        }
        self.into()
    }

    fn poll(
        self,
        cx: &mut Context<'_>,
    ) -> (
        Poll<
            ConnectionHandlerEvent<
                <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundProtocol,
                <EdgeToCoreBlendConnectionHandler as ConnectionHandler>::OutboundOpenInfo,
                ToBehaviour,
            >,
        >,
        ConnectionState,
    );
}

pub struct EdgeToCoreBlendConnectionHandler {
    state: Option<ConnectionState>,
}

impl EdgeToCoreBlendConnectionHandler {
    pub fn new() -> Self {
        Self {
            state: Some(StartingState.into()),
        }
    }
}

#[derive(Debug)]
pub enum FromBehaviour {
    /// Send a message to the other side of the connection.
    Message(Vec<u8>),
}

#[derive(Debug)]
pub enum ToBehaviour {
    /// Notify the behaviour that the message was sent successfully.
    MessageSuccess(Vec<u8>),
    SendError(&'static str),
}

impl ConnectionHandler for EdgeToCoreBlendConnectionHandler {
    type FromBehaviour = FromBehaviour;
    type ToBehaviour = ToBehaviour;
    type InboundProtocol = DeniedUpgrade;
    type InboundOpenInfo = ();
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type OutboundOpenInfo = ();

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        let state = self.state.take().expect("Inconsistent state");
        self.state = Some(state.on_behaviour_event(event));
    }

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        let state = self.state.take().expect("Inconsistent state");
        self.state = Some(state.on_connection_event(event));
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Will refactor this probably by moving the methods to `ConnectionState` itself."
    )]
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

        match state {
            ConnectionState::Starting => Poll::Pending,
            ConnectionState::Dropped(maybe_error_message) => match maybe_error_message {
                Some(error_message) => {
                    self.state = Some(ConnectionState::Dropped(None));
                    Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::SendError(error_message),
                    ))
                }
                None => Poll::Pending,
            },
            ConnectionState::MessageSet { message, .. } => {
                self.state = Some(ConnectionState::MessageSet {
                    message,
                    waker: Some(cx.waker().clone()),
                });
                Poll::Pending
            }
            ConnectionState::ReadyToSend { state, .. } => match state {
                ReadyToSendState::OnlyOutboundStreamSet(outgoing_stream) => {
                    self.state = Some(ConnectionState::ReadyToSend {
                        state: ReadyToSendState::OnlyOutboundStreamSet(outgoing_stream),
                        waker: Some(cx.waker().clone()),
                    });
                    Poll::Pending
                }
                ReadyToSendState::MessageAndOutboundStreamSet(message, outgoing_stream) => {
                    self.state = Some(ConnectionState::Sending {
                        message: message.clone(),
                        outbound_message_send_future: Box::pin(
                            send_msg(outgoing_stream, message).map_ok(|_| ()),
                        ),
                    });
                    Poll::Pending
                }
            },
            ConnectionState::Sending {
                message,
                mut outbound_message_send_future,
            } => {
                let Poll::Ready(message_send_result) = outbound_message_send_future.poll_unpin(cx)
                else {
                    return Poll::Pending;
                };
                if let Err(error) = message_send_result {
                    tracing::error!("Failed to send message. Error {error:?}");
                    self.state = Some(ConnectionState::Dropped(Some("Failed to send message.")));
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    self.state = Some(ConnectionState::Dropped(None));
                    Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::MessageSuccess(message),
                    ))
                }
            }
        }
    }
}
