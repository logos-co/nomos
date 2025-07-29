use core::{
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
};
use std::{collections::VecDeque, io};

use futures::{future::BoxFuture, FutureExt as _};
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{
        handler::{
            ConnectionEvent, DialUpgradeError, FullyNegotiatedInbound, FullyNegotiatedOutbound,
        },
        ConnectionHandlerEvent, SubstreamProtocol,
    },
    Stream, StreamProtocol,
};

use crate::{
    core::with_core::behaviour::handler::conn_maintenance::{
        ConnectionMonitor, ConnectionMonitorOutput,
    },
    recv_msg, send_msg, PROTOCOL_NAME,
};

pub(super) mod conn_maintenance;

#[cfg(feature = "tokio")]
pub(super) mod tokio;

// Metrics
const VALUE_FULLY_NEGOTIATED_INBOUND: &str = "fully_negotiated_inbound";
const VALUE_FULLY_NEGOTIATED_OUTBOUND: &str = "fully_negotiated_outbound";
const VALUE_DIAL_UPGRADE_ERROR: &str = "dial_upgrade_error";
const VALUE_IGNORED: &str = "ignored";

pub struct ConnectionHandler<ConnectionWindowClock> {
    inbound_substream: Option<InboundSubstreamState>,
    outbound_substream: Option<OutboundSubstreamState>,
    outbound_msgs: VecDeque<Vec<u8>>,
    pending_events_to_behaviour: VecDeque<ToBehaviour>,
    monitor: ConnectionMonitor<ConnectionWindowClock>,
    waker: Option<Waker>,
}

type MsgSendFuture = BoxFuture<'static, Result<Stream, io::Error>>;
type MsgRecvFuture = BoxFuture<'static, Result<(Stream, Vec<u8>), io::Error>>;

enum InboundSubstreamState {
    /// A message is being received on the inbound substream.
    PendingRecv(MsgRecvFuture),
    /// A substream has been dropped proactively.
    Dropped,
}

enum OutboundSubstreamState {
    /// A request to open a new outbound substream is being processed.
    PendingOpenSubstream,
    /// An outbound substream is open and ready to send messages.
    Idle(Stream),
    /// A message is being sent on the outbound substream.
    PendingSend(MsgSendFuture),
    /// A substream has been dropped proactively.
    Dropped,
}

impl<ConnectionWindowClock> ConnectionHandler<ConnectionWindowClock> {
    pub const fn new(monitor: ConnectionMonitor<ConnectionWindowClock>) -> Self {
        Self {
            inbound_substream: None,
            outbound_substream: None,
            outbound_msgs: VecDeque::new(),
            pending_events_to_behaviour: VecDeque::new(),
            monitor,
            waker: None,
        }
    }

    /// Mark the inbound/outbound substream state as Dropped.
    /// Then the substream hold by the state will be dropped from memory.
    /// As a result, Swarm will decrease the ref count to the connection,
    /// and close the connection when the count is 0.
    ///
    /// Also, this clears all pending messages and events
    /// to avoid confusions for event recipients.
    fn close_substreams(&mut self) {
        self.inbound_substream = Some(InboundSubstreamState::Dropped);
        self.outbound_substream = Some(OutboundSubstreamState::Dropped);
        self.outbound_msgs.clear();
        self.pending_events_to_behaviour.clear();
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

#[derive(Debug)]
pub enum FromBehaviour {
    /// A message to be sent to the connection.
    Message(Vec<u8>),
    /// Close inbound/outbound substreams.
    /// This happens when [`crate::Behaviour`] determines that one of the
    /// followings is true.
    /// - Max peering degree is reached.
    /// - The peer has been detected as spammy.
    CloseSubstreams,
}

#[derive(Debug)]
pub enum ToBehaviour {
    /// An inbound substream has been successfully upgraded for the blend
    /// protocol.
    FullyNegotiatedInbound,
    /// An outbound substream has been successfully upgraded for the blend
    /// protocol.
    FullyNegotiatedOutbound,
    /// An outbound substream was failed to be upgraded for the blend protocol.
    DialUpgradeError(DialUpgradeError<(), ReadyUpgrade<StreamProtocol>>),
    /// A message has been received from the connection.
    Message(Vec<u8>),
    /// Notifying that the peer is detected as spammy.
    /// The inbound/outbound streams to the peer are closed proactively.
    SpammyPeer,
    /// Notifying that the peer is detected as unhealthy.
    UnhealthyPeer,
    /// Notifying that the peer is detected as healthy.
    HealthyPeer,
    /// An IO error from the connection.
    /// The inbound/outbound streams to the peer are closed proactively.
    IOError(io::Error),
}

impl<ConnectionWindowClock> libp2p::swarm::ConnectionHandler
    for ConnectionHandler<ConnectionWindowClock>
where
    ConnectionWindowClock: futures::Stream<Item = RangeInclusive<u64>> + Unpin + Send + 'static,
{
    type FromBehaviour = FromBehaviour;
    type ToBehaviour = ToBehaviour;
    type InboundProtocol = ReadyUpgrade<StreamProtocol>;
    type InboundOpenInfo = ();
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type OutboundOpenInfo = ();

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ())
    }

    #[expect(deprecated, reason = "Self::OutboundOpenInfo is deprecated")]
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        tracing::info!(gauge.pending_outbound_messages = self.outbound_msgs.len() as u64,);
        tracing::info!(
            gauge.pending_events_to_behaviour = self.pending_events_to_behaviour.len() as u64,
        );

        // Check if the monitor interval has elapsed, if exists.
        // TODO: Refactor this to a separate function.
        if let Poll::Ready(output) = self.monitor.poll(cx) {
            match output {
                Some(ConnectionMonitorOutput::Spammy) => {
                    self.close_substreams();
                    self.pending_events_to_behaviour
                        .push_back(ToBehaviour::SpammyPeer);
                }
                Some(ConnectionMonitorOutput::Unhealthy) => {
                    self.pending_events_to_behaviour
                        .push_back(ToBehaviour::UnhealthyPeer);
                }
                Some(ConnectionMonitorOutput::Healthy) => {
                    self.pending_events_to_behaviour
                        .push_back(ToBehaviour::HealthyPeer);
                }
                None => panic!("Connection monitor stream was closed."),
            }
        }

        // Process pending events to be sent to the behaviour
        if let Some(event) = self.pending_events_to_behaviour.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(event));
        }

        // Process inbound stream
        // TODO: Refactor this to a separate function.
        tracing::debug!("Processing inbound stream");
        match self.inbound_substream.take() {
            None => {
                tracing::debug!("Inbound substream is not initialized yet. Doing nothing.");
            }
            Some(InboundSubstreamState::PendingRecv(mut msg_recv_fut)) => match msg_recv_fut
                .poll_unpin(cx)
            {
                Poll::Ready(Ok((stream, msg))) => {
                    tracing::debug!(
                        "Received message from inbound stream. Notifying behaviour if necessary..."
                    );

                    // Record the message to the monitor.
                    self.monitor.record_message();

                    self.inbound_substream =
                        Some(InboundSubstreamState::PendingRecv(recv_msg(stream).boxed()));

                    // Notify behaviour.
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::Message(msg),
                    ));
                }
                Poll::Ready(Err(e)) => {
                    tracing::error!(
                        "Failed to receive message from inbound stream: {e:?}. Dropping both inbound/outbound substreams"
                    );
                    self.close_substreams();
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        ToBehaviour::IOError(e),
                    ));
                }
                Poll::Pending => {
                    tracing::debug!("No message received from inbound stream yet. Waiting more...");
                    self.inbound_substream = Some(InboundSubstreamState::PendingRecv(msg_recv_fut));
                }
            },
            Some(InboundSubstreamState::Dropped) => {
                tracing::debug!("Inbound substream has been dropped proactively. Doing nothing.");
                self.inbound_substream = Some(InboundSubstreamState::Dropped);
            }
        }

        // Process outbound stream
        // TODO: Refactor this to a separate function.
        tracing::debug!("Processing outbound stream");
        loop {
            match self.outbound_substream.take() {
                // If the request to open a new outbound substream is still being processed, wait
                // more.
                Some(OutboundSubstreamState::PendingOpenSubstream) => {
                    self.outbound_substream = Some(OutboundSubstreamState::PendingOpenSubstream);
                    self.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
                // If the substream is idle, and if it's time to send a message, send it.
                Some(OutboundSubstreamState::Idle(stream)) => {
                    if let Some(msg) = self.outbound_msgs.pop_front() {
                        tracing::debug!("Sending message to outbound stream: {:?}", msg);
                        self.outbound_substream = Some(OutboundSubstreamState::PendingSend(
                            send_msg(stream, msg).boxed(),
                        ));
                    } else {
                        tracing::debug!("Nothing to send to outbound stream");
                        self.outbound_substream = Some(OutboundSubstreamState::Idle(stream));
                        self.waker = Some(cx.waker().clone());
                        return Poll::Pending;
                    }
                }
                // If a message is being sent, check if it's done.
                Some(OutboundSubstreamState::PendingSend(mut msg_send_fut)) => {
                    match msg_send_fut.poll_unpin(cx) {
                        Poll::Ready(Ok(stream)) => {
                            tracing::debug!("Message sent to outbound stream");
                            self.outbound_substream = Some(OutboundSubstreamState::Idle(stream));
                        }
                        Poll::Ready(Err(e)) => {
                            tracing::error!("Failed to send message to outbound stream: {e:?}. Dropping both inbound and outbound substreams");
                            self.close_substreams();
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                ToBehaviour::IOError(e),
                            ));
                        }
                        Poll::Pending => {
                            self.outbound_substream =
                                Some(OutboundSubstreamState::PendingSend(msg_send_fut));
                            self.waker = Some(cx.waker().clone());
                            return Poll::Pending;
                        }
                    }
                }
                Some(OutboundSubstreamState::Dropped) => {
                    tracing::debug!("Outbound substream has been dropped proactively");
                    self.outbound_substream = Some(OutboundSubstreamState::Dropped);
                    return Poll::Pending;
                }
                // If there is no outbound substream, request to open a new one.
                None => {
                    self.outbound_substream = Some(OutboundSubstreamState::PendingOpenSubstream);
                    return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                        protocol: SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ()),
                    });
                }
            }
        }
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        match event {
            FromBehaviour::Message(msg) => {
                self.outbound_msgs.push_back(msg);
            }
            FromBehaviour::CloseSubstreams => {
                self.close_substreams();
            }
        }
    }

    #[expect(
        deprecated,
        reason = "Self::InboundOpenInfo and Self::OutboundOpenInfo are deprecated"
    )]
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        let event_name = match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream,
                ..
            }) => {
                tracing::debug!("FullyNegotiatedInbound: Creating inbound substream");
                self.inbound_substream =
                    Some(InboundSubstreamState::PendingRecv(recv_msg(stream).boxed()));
                self.pending_events_to_behaviour
                    .push_back(ToBehaviour::FullyNegotiatedInbound);
                VALUE_FULLY_NEGOTIATED_INBOUND
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream,
                ..
            }) => {
                tracing::debug!("FullyNegotiatedOutbound: Creating outbound substream");
                self.outbound_substream = Some(OutboundSubstreamState::Idle(stream));
                self.pending_events_to_behaviour
                    .push_back(ToBehaviour::FullyNegotiatedOutbound);
                VALUE_FULLY_NEGOTIATED_OUTBOUND
            }
            ConnectionEvent::DialUpgradeError(e) => {
                tracing::error!("DialUpgradeError: {:?}", e);
                self.pending_events_to_behaviour
                    .push_back(ToBehaviour::DialUpgradeError(e));
                self.close_substreams();
                VALUE_DIAL_UPGRADE_ERROR
            }
            event => {
                tracing::debug!("Ignoring connection event: {:?}", event);
                VALUE_IGNORED
            }
        };

        tracing::info!(counter.connection_event = 1, event = event_name);
        self.try_wake();
    }
}
