use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
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
use nomos_blend_scheduling::membership::Membership;

use crate::core::with_edge::behaviour::handler::{ConnectionHandler, ToBehaviour};

mod handler;

const LOG_TARGET: &str = "blend::network::core::edge::behaviour";

#[derive(Debug)]
pub enum Event {
    /// A message received from an edge peer.
    Message(Vec<u8>),
}

#[derive(Debug)]
pub struct Config {
    pub connection_timeout: Duration,
    pub max_incoming_connections: usize,
}

struct PeeringDegreeInfo {
    current_incoming: usize,
    max_incoming: usize,
}

impl PeeringDegreeInfo {
    const fn new(max_incoming: usize) -> Self {
        Self {
            current_incoming: 0,
            max_incoming,
        }
    }

    const fn remaining_incoming_capacity(&self) -> usize {
        self.max_incoming.saturating_sub(self.current_incoming)
    }
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
    connected_edge_peers: HashMap<PeerId, HashSet<ConnectionId>>,
    peering_degree_info: PeeringDegreeInfo,
}

impl Behaviour {
    #[must_use]
    pub fn new(config: &Config, current_membership: Option<Membership<PeerId>>) -> Self {
        Self {
            events: VecDeque::new(),
            waker: None,
            current_membership,
            connection_timeout: config.connection_timeout,
            connected_edge_peers: HashMap::new(),
            peering_degree_info: PeeringDegreeInfo::new(config.max_incoming_connections),
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    const fn can_accept_incoming_connection(&self) -> bool {
        self.peering_degree_info.remaining_incoming_capacity() > 0
    }

    fn add_connection_with_peer(&mut self, peer_id: PeerId, connection_id: ConnectionId) -> bool {
        let entry = self.connected_edge_peers.entry(peer_id);
        match entry {
            Entry::Occupied(mut existing_connection_set) => {
                existing_connection_set.get_mut().insert(connection_id)
            }
            Entry::Vacant(new_connection_set) => {
                new_connection_set.insert(vec![connection_id].into_iter().collect());
                self.peering_degree_info.current_incoming = self
                    .peering_degree_info
                    .current_incoming
                    .checked_add(1)
                    .unwrap();
                false
            }
        }
    }

    fn remove_connection_with_peer(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
    ) -> bool {
        let Entry::Occupied(mut entry) = self.connected_edge_peers.entry(peer_id) else {
            return false;
        };
        entry.get_mut().remove(&connection_id);
        if entry.get().is_empty() {
            entry.remove_entry();
            self.peering_degree_info.current_incoming = self
                .peering_degree_info
                .current_incoming
                .checked_sub(1)
                .unwrap();
        }
        true
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Either<ConnectionHandler, DummyConnectionHandler>;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if !self.can_accept_incoming_connection() {
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
            self.remove_connection_with_peer(peer_id, connection_id);
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {
            Either::Left(ToBehaviour::Message(message)) => {
                self.events
                    .push_back(ToSwarm::GenerateEvent(Event::Message(message)));
            }
            Either::Left(ToBehaviour::SubstreamOpened) => {
                if self.add_connection_with_peer(peer, connection_id) {
                    tracing::warn!(target: LOG_TARGET, "There should not be an entry stored for this peer ID {peer:?} and connection ID {connection_id:?}.");
                }
            }
            Either::Left(ToBehaviour::SubstreamClosed(error)) => {
                tracing::trace!(target: LOG_TARGET, "Substream with peer ID {peer:?} and connection ID: {connection_id:?} closed with error: {error:?}.");
                if !self.remove_connection_with_peer(peer, connection_id) {
                    tracing::warn!(target: LOG_TARGET, "Closing a substream that was not previously added to the map of open substreams. Peer ID: {peer:?}, connection ID: {connection_id:?}.");
                }
            }
        }
        self.try_wake();
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
