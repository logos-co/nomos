use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use std::io;

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{
    core::upgrade::{DeniedUpgrade, ReadyUpgrade},
    swarm::{
        handler::{ConnectionEvent, FullyNegotiatedInbound, InboundUpgradeSend},
        ConnectionHandler, ConnectionHandlerEvent, SubstreamProtocol,
    },
    StreamProtocol,
};
use tokio::time::sleep;

use crate::handler::{recv_msg, PROTOCOL_NAME};

const LOG_TARGET: &str = "blend::libp2p::handler::core-edge";

type TimerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type MessageReceiveFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, io::Error>> + Send>>;

enum ConnectionState {
    Starting {
        connection_timeout: Duration,
    },
    ReadyToReceive {
        timeout_timer: TimerFuture,
        incoming_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
    },
    Receiving {
        timeout_timer: TimerFuture,
        incoming_message: MessageReceiveFuture,
    },
    Dropped,
}

pub struct CoreToEdgeBlendConnectionHandler {
    state: Option<ConnectionState>,
}

impl CoreToEdgeBlendConnectionHandler {
    pub const fn new(connection_timeout: Duration) -> Self {
        Self {
            state: Some(ConnectionState::Starting { connection_timeout }),
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
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: incoming_stream,
                ..
            }) => {
                tracing::debug!(target: LOG_TARGET, "Fully negotiated inbound connection. Starting timer and initializing incoming stream.");
                let state = self.state.take().expect("Inconsistent state");

                let ConnectionState::Starting { connection_timeout } = state else {
                    tracing::trace!(target: LOG_TARGET, "Connection handler not in the expected `Starting` state.");
                    self.state = Some(ConnectionState::Dropped);
                    return;
                };
                self.state = Some(ConnectionState::ReadyToReceive {
                    timeout_timer: Box::pin(sleep(connection_timeout)),
                    incoming_stream,
                });
            }
            ConnectionEvent::ListenUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Inbound upgrade error: {error:?}");
                self.state = Some(ConnectionState::Dropped);
            }
            event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {event:?}");
            }
        }
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

        match state {
            ConnectionState::Starting { .. } | ConnectionState::Dropped => Poll::Pending,
            ConnectionState::ReadyToReceive {
                mut timeout_timer,
                incoming_stream,
            } => {
                let Poll::Pending = timeout_timer.poll_unpin(cx) else {
                    tracing::debug!(target: LOG_TARGET, "Timeout reached without starting the reception of the message. Closing the connection.");
                    self.state = Some(ConnectionState::Dropped);
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::FailedReception,
                    ));
                };
                self.state = Some(ConnectionState::Receiving {
                    timeout_timer,
                    incoming_message: Box::pin(
                        recv_msg(incoming_stream).map_ok(|(_, message)| message),
                    ),
                });
                Poll::Pending
            }
            ConnectionState::Receiving {
                mut timeout_timer,
                mut incoming_message,
            } => {
                let Poll::Pending = timeout_timer.poll_unpin(cx) else {
                    tracing::debug!(target: LOG_TARGET, "Timeout reached without completing the reception of the message. Closing the connection.");
                    self.state = Some(ConnectionState::Dropped);
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::FailedReception,
                    ));
                };
                let Poll::Ready(message_receive_result) = incoming_message.poll_unpin(cx) else {
                    return Poll::Pending;
                };
                match message_receive_result {
                    Err(error) => {
                        tracing::error!("Failed to receive message. Error {error:?}");
                        Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            ToBehaviour::FailedReception,
                        ))
                    }
                    Ok(message) => Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::Message(message),
                    )),
                }
            }
        }
    }
}
