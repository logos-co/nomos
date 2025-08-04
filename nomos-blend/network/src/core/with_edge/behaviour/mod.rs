use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    task::{Context, Poll, Waker},
    time::Duration,
};

use either::Either;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler as DummyConnectionHandler, ConnectionClosed, ConnectionDenied,
        ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent, THandlerOutEvent,
        ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_scheduling::{deserialize_encapsulated_message, membership::Membership};

use crate::{
    core::with_edge::behaviour::handler::{ConnectionHandler, ToBehaviour},
    message::ValidateMessagePublicHeader as _,
    EncapsulatedMessageWithValidatedPublicHeader,
};

mod handler;

const LOG_TARGET: &str = "blend::network::core::edge::behaviour";

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the edge peers, after its public header
    /// has been verified.
    Message(EncapsulatedMessageWithValidatedPublicHeader),
}

#[derive(Debug)]
pub struct Config {
    pub connection_timeout: Duration,
    pub max_incoming_connections: usize,
}

/// A [`NetworkBehaviour`]:
/// - receives messages from edge nodes and forwards them to the swarm.
pub struct Behaviour {
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<Infallible, Infallible>>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    // TODO: Replace with the session stream and make this a non-Option
    current_membership: Option<Membership<PeerId>>,
    // Timeout to close connection with an edge node if a message is not received on time.
    connection_timeout: Duration,
    connected_edge_peers: HashSet<(PeerId, ConnectionId)>,
    max_incoming_connections: usize,
}

impl Behaviour {
    #[must_use]
    pub fn new(config: &Config, current_membership: Option<Membership<PeerId>>) -> Self {
        Self {
            events: VecDeque::new(),
            waker: None,
            current_membership,
            connection_timeout: config.connection_timeout,
            connected_edge_peers: HashSet::with_capacity(config.max_incoming_connections),
            max_incoming_connections: config.max_incoming_connections,
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn handle_received_serialized_encapsulated_message(&mut self, serialized_message: &[u8]) {
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            return;
        };

        let Ok(validated_message) = deserialized_encapsulated_message.validate_public_header()
        else {
            return;
        };

        self.events
            .push_back(ToSwarm::GenerateEvent(Event::Message(validated_message)));
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Either<ConnectionHandler, DummyConnectionHandler>;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.connected_edge_peers.insert((peer, connection_id));
        let connected_peers = self.connected_edge_peers.len();

        // If the new peer makes the set of incoming connections too large, do not try
        // to upgrade the connection.
        if connected_peers > self.max_incoming_connections {
            tracing::trace!(target: LOG_TARGET, "Connected peer {peer:?} on connection {connection_id:?} will not be upgraded since we are already at maximum incoming connection capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        let Some(membership) = &self.current_membership else {
            return Ok(Either::Right(DummyConnectionHandler));
        };
        // Allow only inbound connections from edge nodes.
        Ok(if membership.contains_remote(&peer) {
            Either::Right(DummyConnectionHandler)
        } else {
            Either::Left(ConnectionHandler::new(self.connection_timeout))
        })
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // No outbound sub-stream at all, since substreams with core nodes are handled
        // elsewhere, and substreams with edge nodes are not allowed.
        Ok(Either::Right(DummyConnectionHandler))
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed {
            peer_id,
            connection_id,
            ..
        }) = event
        {
            self.connected_edge_peers.remove(&(peer_id, connection_id));
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        if let Either::Left(ToBehaviour::Message(message)) = event {
            self.handle_received_serialized_encapsulated_message(&message);
            self.try_wake();
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(event) = self.events.pop_front() {
            Poll::Ready(event)
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
