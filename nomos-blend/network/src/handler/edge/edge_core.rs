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

use crate::handler::send_msg;

const LOG_TARGET: &str = "blend::libp2p::handler::edge-core";

type MessageSendFuture = Pin<Box<dyn Future<Output = Result<(), io::Error>> + Send>>;

enum ReadyToSendState {
    OnlyOutboundStreamSet(<ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output),
    MessageAndOutboundStreamSet(
        Vec<u8>,
        <ReadyUpgrade<StreamProtocol> as OutboundUpgradeSend>::Output,
    ),
}

enum ConnectionState {
    Starting,
    MessageSet {
        message: Vec<u8>,
        waker: Option<Waker>,
    },
    ReadyToSend {
        state: ReadyToSendState,
        waker: Option<Waker>,
    },
    Sending {
        message: Vec<u8>,
        outbound_message_send_future: MessageSendFuture,
    },
    Dropped(Option<&'static str>),
}

pub struct EdgeToCoreBlendConnectionHandler {
    state: Option<ConnectionState>,
}

impl EdgeToCoreBlendConnectionHandler {
    pub const fn new() -> Self {
        Self {
            state: Some(ConnectionState::Starting),
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
        let FromBehaviour::Message(message_to_send) = event;
        let state = self.state.take().expect("Inconsistent state");

        match state {
            ConnectionState::Starting => {
                self.state = Some(ConnectionState::MessageSet {
                    message: message_to_send,
                    waker: None,
                });
            }
            ConnectionState::ReadyToSend {
                state: ReadyToSendState::OnlyOutboundStreamSet(outgoing_stream),
                waker,
            } => {
                self.state = Some(ConnectionState::ReadyToSend {
                    state: ReadyToSendState::MessageAndOutboundStreamSet(
                        message_to_send,
                        outgoing_stream,
                    ),
                    waker: None,
                });
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
            _ => {
                tracing::debug!(target: LOG_TARGET, "Received a new `Message` event from behaviour when only a single message can be sent.");
            }
        }
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
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: outgoing_stream,
                ..
            }) => {
                tracing::debug!(target: LOG_TARGET, "Fully negotiated outbound connection. Initializing outgoing stream.");
                let state = self.state.take().expect("Inconsistent state");

                match state {
                    ConnectionState::Starting => {
                        self.state = Some(ConnectionState::ReadyToSend {
                            state: ReadyToSendState::OnlyOutboundStreamSet(outgoing_stream),
                            waker: None,
                        });
                    }
                    ConnectionState::MessageSet { message, waker } => {
                        self.state = Some(ConnectionState::ReadyToSend {
                            state: ReadyToSendState::MessageAndOutboundStreamSet(
                                message,
                                outgoing_stream,
                            ),
                            waker: None,
                        });
                        if let Some(waker) = waker {
                            waker.wake();
                        }
                    }
                    _ => {
                        tracing::trace!(target: LOG_TARGET, "Outbound channel negotiated with an inconsistent internal state.");
                    }
                }
            }
            ConnectionEvent::DialUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Outbound upgrade error: {error:?}");
                let old_state = self.state.take().expect("Inconsistent state");
                self.state = Some(ConnectionState::Dropped(Some("DialUpgradeError")));
                if let ConnectionState::MessageSet {
                    waker: Some(waker), ..
                } = old_state
                {
                    waker.wake();
                }
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
