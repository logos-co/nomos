use core::num::NonZeroUsize;
use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    mem,
    task::{Context, Poll, Waker},
    time::Duration,
};

use either::Either;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler as DummyConnectionHandler, ConnectionClosed, ConnectionDenied,
        ConnectionId, FromSwarm, NetworkBehaviour, NotifyHandler, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, StreamProtocol,
};
use nomos_blend_message::encap::{self, encapsulated::PoQVerificationInputMinusSigningKey};
use nomos_blend_scheduling::{
    deserialize_encapsulated_message, membership::Membership,
    message_blend::crypto::UnwrappedEncapsulatedMessage,
};

use crate::core::with_edge::behaviour::handler::{ConnectionHandler, FromBehaviour, ToBehaviour};

mod handler;

#[cfg(test)]
mod tests;

const LOG_TARGET: &str = "blend::network::core::edge::behaviour";

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the edge peers, after its public header
    /// has been verified.
    Message(UnwrappedEncapsulatedMessage),
}

#[derive(Debug)]
pub struct Config {
    pub connection_timeout: Duration,
    pub max_incoming_connections: usize,
    pub minimum_network_size: NonZeroUsize,
}

/// A [`NetworkBehaviour`]:
/// - receives messages from edge nodes and forwards them to the swarm.
pub struct Behaviour<ProofsVerifier> {
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    // TODO: Make this a non-Option by refactoring tests
    current_membership: Option<Membership<PeerId>>,
    // Timeout to close connection with an edge node if a message is not received on time.
    connection_timeout: Duration,
    upgraded_edge_peers: HashSet<(PeerId, ConnectionId)>,
    max_incoming_connections: usize,
    protocol_name: StreamProtocol,
    minimum_network_size: NonZeroUsize,
    poq_verification_inputs: PoQVerificationInputMinusSigningKey,
    poq_verifier: ProofsVerifier,
}

impl<ProofsVerifier> Behaviour<ProofsVerifier> {
    #[must_use]
    pub fn new(
        config: &Config,
        current_membership: Option<Membership<PeerId>>,
        protocol_name: StreamProtocol,
        poq_verification_inputs: PoQVerificationInputMinusSigningKey,
        poq_verifier: ProofsVerifier,
    ) -> Self {
        Self {
            events: VecDeque::new(),
            waker: None,
            current_membership,
            connection_timeout: config.connection_timeout,
            upgraded_edge_peers: HashSet::with_capacity(config.max_incoming_connections),
            max_incoming_connections: config.max_incoming_connections,
            protocol_name,
            minimum_network_size: config.minimum_network_size,
            poq_verification_inputs,
            poq_verifier,
        }
    }

    pub fn start_new_session(&mut self, new_membership: Membership<PeerId>) {
        self.current_membership = Some(new_membership);
        // Close all the connections without waiting for the transition period,
        // so that edge nodes can retry with the new membership.
        let peers = mem::take(&mut self.upgraded_edge_peers);
        for conn in &peers {
            self.close_substream(*conn);
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    #[must_use]
    fn available_connection_slots(&self) -> usize {
        self.max_incoming_connections
            .saturating_sub(self.upgraded_edge_peers.len())
    }

    fn handle_negotiated_connection(&mut self, connection: (PeerId, ConnectionId)) {
        // We need to check if we still have available connection slots, as it
        // is possible, especially upon session transition, that more
        // than the maximum allowed number of peers are trying to
        // connect to us. So once we stream is actually upgraded, we
        // downgrade it again if we do not have space left for it. This will
        // most likely, depending on the swarm configuration, result in the
        // connection being dropped.
        if self.available_connection_slots() == 0 {
            tracing::debug!(target: LOG_TARGET, "Connection {connection:?} must be closed because peering degree limit has been reached.");
            self.close_substream(connection);
            return;
        }
        tracing::debug!(target: LOG_TARGET, "Connection {connection:?} has been negotiated.");
        self.events.push_back(ToSwarm::NotifyHandler {
            peer_id: connection.0,
            handler: NotifyHandler::One(connection.1),
            event: Either::Left(FromBehaviour::StartReceiving),
        });
        self.try_wake();
        self.upgraded_edge_peers.insert(connection);
    }

    fn close_substream(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        self.events.push_back(ToSwarm::NotifyHandler {
            peer_id,
            handler: NotifyHandler::One(connection_id),
            event: Either::Left(FromBehaviour::CloseSubstream),
        });
        self.try_wake();
    }

    fn is_network_large_enough(&self) -> bool {
        self.current_membership.as_ref().map_or(1, Membership::size)
            >= self.minimum_network_size.get()
    }
}

impl<ProofsVerifier> Behaviour<ProofsVerifier>
where
    ProofsVerifier: encap::ProofsVerifier,
{
    fn handle_received_serialized_encapsulated_message(&mut self, serialized_message: &[u8]) {
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            tracing::trace!(target: LOG_TARGET, "Failed to deserialize received message. Ignoring...");
            return;
        };

        let Ok(validated_message) = deserialized_encapsulated_message
            .verify_and_unwrap_public_header(&self.poq_verification_inputs, &self.poq_verifier)
        else {
            tracing::trace!(target: LOG_TARGET, "Failed to validate public header of received message. Ignoring...");
            return;
        };

        self.events
            .push_back(ToSwarm::GenerateEvent(Event::Message(validated_message)));
        self.try_wake();
    }
}

impl<ProofsVerifier> NetworkBehaviour for Behaviour<ProofsVerifier>
where
    ProofsVerifier: encap::ProofsVerifier + 'static,
{
    type ConnectionHandler = Either<ConnectionHandler, DummyConnectionHandler>;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If the new peer makes the set of incoming connections too large, do not try
        // to upgrade the connection.
        if self.upgraded_edge_peers.len() >= self.max_incoming_connections {
            tracing::trace!(target: LOG_TARGET, "Connected peer {peer:?} on connection {connection_id:?} will not be upgraded since we are already at maximum incoming connection capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        let Some(membership) = &self.current_membership else {
            return Ok(Either::Right(DummyConnectionHandler));
        };
        // Allow only inbound connections from edge nodes, if the Blend network is large
        // enough.
        Ok(
            if membership.contains(&peer) || !self.is_network_large_enough() {
                Either::Right(DummyConnectionHandler)
            } else {
                Either::Left(ConnectionHandler::new(
                    self.connection_timeout,
                    self.protocol_name.clone(),
                ))
            },
        )
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
            self.upgraded_edge_peers.remove(&(peer_id, connection_id));
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {
            Either::Left(ToBehaviour::Message(message)) => {
                self.handle_received_serialized_encapsulated_message(&message);
            }
            Either::Left(ToBehaviour::SubstreamOpened) => {
                self.handle_negotiated_connection((peer_id, connection_id));
            }
            Either::Left(_) | Either::Right(_) => {}
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
