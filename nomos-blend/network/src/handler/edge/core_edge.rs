use std::{
    future::Future,
    io,
    task::{Context, Poll, Waker},
    time::Duration,
};

use libp2p::{
    core::upgrade::{DeniedUpgrade, ReadyUpgrade},
    swarm::{
        handler::{ConnectionEvent, FullyNegotiatedInbound, InboundUpgradeSend},
        ConnectionHandler, ConnectionHandlerEvent, SubstreamProtocol,
    },
    StreamProtocol,
};
use tokio::time::sleep;

use crate::handler::PROTOCOL_NAME;

const LOG_TARGET: &str = "blend::libp2p::handler::core-edge";

struct StreamDetails {
    timeout_timer: Box<dyn Future<Output = ()> + Send>,
    incoming_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
}

pub struct CoreToEdgeBlendConnectionHandler {
    received_message: Option<Vec<u8>>,
    timeout: Duration,
    stream_details: Option<StreamDetails>,
    waker: Option<Waker>,
}

impl CoreToEdgeBlendConnectionHandler {
    pub const fn new(connection_timeout: Duration) -> Self {
        Self {
            received_message: None,
            timeout: connection_timeout,
            stream_details: None,
            waker: None,
        }
    }
}

#[derive(Debug)]
pub enum ToBehaviour {
    /// A message has been received from the connection.
    Message(Vec<u8>),
    /// An IO error from the connection.
    /// The inbound/outbound streams to the peer are closed proactively.
    IOError(io::Error),
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
                self.stream_details = Some(StreamDetails {
                    timeout_timer: Box::new(sleep(self.timeout)),
                    incoming_stream,
                })
            }
            ConnectionEvent::ListenUpgradeError(error) => {
                tracing::debug!(target: LOG_TARGET, "Inbound upgrade error: {error:?}");
            }
            event => {
                tracing::debug!(target: LOG_TARGET, "Ignoring connection event {event:?}");
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        todo!()
    }
}
