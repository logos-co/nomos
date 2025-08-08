use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
};

use either::Either;
use futures::Stream;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler as DummyConnectionHandler, ConnectionClosed, ConnectionDenied,
        ConnectionId, FromSwarm, NetworkBehaviour, NotifyHandler, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_message::MessageIdentifier;
use nomos_blend_scheduling::{
    deserialize_encapsulated_message, membership::Membership, serialize_encapsulated_message,
    EncapsulatedMessage,
};

use crate::{
    core::with_core::{
        behaviour::handler::{
            conn_maintenance::ConnectionMonitor, ConnectionHandler, FromBehaviour, ToBehaviour,
        },
        error::Error,
    },
    message::{EncapsulatedMessageWithValidatedPublicHeader, ValidateMessagePublicHeader as _},
};

mod handler;

#[cfg(feature = "tokio")]
pub use self::handler::tokio::ObservationWindowTokioIntervalProvider;

const LOG_TARGET: &str = "blend::network::core::core::behaviour";

#[derive(Debug)]
pub struct Config {
    /// The [minimum, maximum] peering degree of this node.
    pub peering_degree: RangeInclusive<usize>,
}

#[derive(Debug, Clone, Copy)]
struct PeerConnectionDetails {
    /// Role of the remote peer in this connection.
    role: Endpoint,
    /// Latest negotiated state of the peer.
    negotiated_state: NegotiatedPeerState,
    connection_id: ConnectionId,
}

/// A [`NetworkBehaviour`] that processes incoming Blend messages, and
/// propagates messages from the Blend service to the rest of the Blend network.
///
/// The public header and uniqueness of incoming messages is validated according to the [Blend v1 specification](https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084) before the message is propagated to the swarm and to the Blend service.
/// The same checks are applied to messages received by the Blend service before
/// they are propagated to the rest of the network, making sure no peer marks
/// this node as malicious due to an invalid Blend message.
pub struct Behaviour<ObservationWindowClockProvider> {
    /// Tracks connections between this node and other core nodes.
    ///
    /// Only connections with other core nodes that are established before the
    /// specified connection limit is reached will be upgraded and the state of
    /// the peer negotiated, monitored, and reported to the swarm.
    negotiated_peers: HashMap<PeerId, PeerConnectionDetails>,
    pending_connections: HashMap<(PeerId, ConnectionId), Endpoint>,
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    /// The session-bound storage keeping track, for each peer, what message
    /// identifiers have been exchanged between them.
    /// Sending a message with the same identifier more than once results in
    /// the peer being flagged as malicious, and the connection dropped.
    // TODO: This cache should be cleared after the session transition period has
    // passed.
    exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    observation_window_clock_provider: ObservationWindowClockProvider,
    // TODO: Replace with the session stream and make this a non-Option
    current_membership: Option<Membership<PeerId>>,
    /// The [minimum, maximum] peering degree of this node.
    peering_degree: RangeInclusive<usize>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum NegotiatedPeerState {
    Healthy,
    Unhealthy,
    Spammy,
}

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the core peers, after its public header
    /// has been verified.
    Message(Box<EncapsulatedMessageWithValidatedPublicHeader>, PeerId),
    /// A peer on a given connection has been detected as unhealthy.
    UnhealthyPeer(PeerId),
    /// A peer on a given connection that was previously unhealthy has returned
    /// to a healthy state.
    HealthyPeer(PeerId),
    /// A connection with a peer has dropped. The last state that was negotiated
    /// with the peer is also returned.
    PeerDisconnected(PeerId, NegotiatedPeerState),
}

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider> {
    #[must_use]
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
    ) -> Self {
        Self {
            negotiated_peers: HashMap::with_capacity(*config.peering_degree.end()),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::with_capacity(
                current_membership
                    .as_ref()
                    .map(Membership::size)
                    .unwrap_or_default(),
            ),
            observation_window_clock_provider,
            current_membership,
            peering_degree: config.peering_degree.clone(),
            pending_connections: HashMap::new(),
        }
    }

    /// Publish an already-encapsulated message to all connected peers.
    ///
    /// Before the message is propagated, its public header is validated to make
    /// sure the receiving peer won't mark us as malicious.
    pub fn validate_and_publish_message(
        &mut self,
        message: EncapsulatedMessage,
    ) -> Result<(), Error> {
        let validated_message = message
            .validate_public_header()
            .map_err(|_| Error::InvalidMessage)?;
        self.forward_validated_message_and_maybe_exclude(&validated_message, None)?;
        Ok(())
    }

    /// Publish an already-encapsulated message to all connected peers.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    pub fn publish_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
    ) -> Result<(), Error> {
        self.forward_validated_message_and_maybe_exclude(message, None)?;
        Ok(())
    }

    /// Forwards a message to all healthy connections except the specified one.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers that support
    /// the blend protocol.
    pub fn forward_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        excluded_peer: PeerId,
    ) -> Result<(), Error> {
        self.forward_validated_message_and_maybe_exclude(message, Some(excluded_peer))?;
        Ok(())
    }

    #[must_use]
    pub fn num_healthy_peers(&self) -> usize {
        self.negotiated_peers
            .values()
            .filter(|state| state.negotiated_state == NegotiatedPeerState::Healthy)
            .count()
    }

    pub const fn minimum_healthy_peering_degree(&self) -> usize {
        *self.peering_degree.start()
    }

    #[must_use]
    pub fn available_connection_slots(&self) -> usize {
        self.peering_degree
            .end()
            .saturating_sub(self.negotiated_peers.len())
    }

    /// Forwards a message to all connected and healthy peers except the
    /// excluded peer.
    ///
    /// For each potential recipient, a uniqueness check is performed to avoid
    /// sending a duplicate message to a peer and be marked as malicious by
    /// them.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers
    /// that support the blend protocol or that have not yet received the
    /// message.
    fn forward_validated_message_and_maybe_exclude(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        excluded_peer: Option<PeerId>,
    ) -> Result<(), Error> {
        let message_id = message.id();

        let serialized_message = serialize_encapsulated_message(message);
        let mut num_peers = 0u32;
        self.negotiated_peers
            .iter()
            // Exclude the connection the message was received from.
            .filter(|(peer_id, _)| (excluded_peer != Some(**peer_id)))
            // Exclude from the list of candidate peers any peer that is not in a healthy state.
            .filter(|(_, peer_state)| peer_state.negotiated_state == NegotiatedPeerState::Healthy)
            .for_each(|(peer_id, peer_state)| {
                if self
                    .exchanged_message_identifiers
                    .entry(*peer_id)
                    .or_default()
                    .insert(message_id)
                {
                    tracing::debug!(target: LOG_TARGET, "Registering event for peer {peer_id:?} to send msg");
                    self.events.push_back(ToSwarm::NotifyHandler {
                        peer_id: *peer_id,
                        handler: NotifyHandler::One(peer_state.connection_id),
                        event: Either::Left(FromBehaviour::Message(serialized_message.clone())),
                    });
                    num_peers += 1;
                } else {
                    tracing::trace!(target: LOG_TARGET, "Not sending message to peer {peer_id:?} because we already exchanged this message with them.");
                }
            });

        if num_peers == 0 {
            Err(Error::NoPeers)
        } else {
            self.try_wake();
            Ok(())
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn handle_negotiated_connection(&mut self, connection: (PeerId, ConnectionId)) {
        // We need to check if we still have available connection slots, as it is
        // possible, especially upon session transition, that more than the maximum
        // allowed number of peers are trying to connect to us. So once the stream is
        // actually upgraded, we downgrade it again if we do not have space left for it.
        // By not adding the new connection to the map of negotiated peers, the swarm
        // will not be notified about this dropped connection, which is what we want.
        if self.available_connection_slots() == 0 {
            tracing::debug!(target: LOG_TARGET, "Connection {:?} with peer {:?} must be closed because peering degree limit has been reached.", connection.1, connection.0);
            self.events.push_back(ToSwarm::NotifyHandler {
                peer_id: connection.0,
                handler: NotifyHandler::One(connection.1),
                event: Either::Left(FromBehaviour::CloseSubstreams),
            });
            self.try_wake();
            return;
        }
        tracing::debug!(target: LOG_TARGET, "Connection {:?} with peer {:?} has been negotiated.", connection.1, connection.0);
        let Some(peer_role) = self.pending_connections.remove(&connection) else {
            #[cfg(debug_assertions)]
            panic!(
                "Negotiated connection with peer {:?} not found in storage.",
                connection.0
            );
            #[cfg(not(debug_assertions))]
            {
                tracing::warn!(target: LOG_TARGET, "Negotiated connection not found in storage. Ignoring.");
                return;
            }
        };
        debug_assert!(!self.negotiated_peers.contains_key(&connection.0));
        self.negotiated_peers.insert(
            connection.0,
            PeerConnectionDetails {
                role: peer_role,
                negotiated_state: NegotiatedPeerState::Healthy,
                connection_id: connection.1,
            },
        );
    }

    fn handle_spammy_connection(&mut self, connection: (PeerId, ConnectionId)) {
        tracing::debug!(target: LOG_TARGET, "Marking peer {:?} as malicious.", connection.0);
        let Some(peer_details) = self.negotiated_peers.get_mut(&connection.0) else {
            #[cfg(debug_assertions)]
            panic!(
                "Connection with peer {:?} not found in storage.",
                connection.0
            );
            #[cfg(not(debug_assertions))]
            {
                tracing::warn!(target: LOG_TARGET, "Connection with peer {:?} not found in storage. Ignoring.", connection.0);
                return;
            }
        };
        debug_assert!(peer_details.connection_id == connection.1);
        peer_details.negotiated_state = NegotiatedPeerState::Spammy;
    }

    /// Mark the connection with the sender of a malformed message as malicious
    /// and instruct the connection handler to drop the substream.
    fn mark_connection_as_malicious(&mut self, connection: (PeerId, ConnectionId)) {
        tracing::debug!(target: LOG_TARGET, "Closing connection with malicious peer {:?}.", connection.0);
        self.handle_spammy_connection(connection);
        self.events.push_back(ToSwarm::NotifyHandler {
            peer_id: connection.0,
            handler: NotifyHandler::One(connection.1),
            event: Either::Left(FromBehaviour::CloseSubstreams),
        });
        self.try_wake();
    }

    fn handle_unhealthy_connection(&mut self, connection: (PeerId, ConnectionId)) {
        let Some(peer_details) = self.negotiated_peers.get_mut(&connection.0) else {
            #[cfg(debug_assertions)]
            panic!(
                "Connection with peer {:?} not found in storage.",
                connection.0
            );
            #[cfg(not(debug_assertions))]
            {
                tracing::warn!(target: LOG_TARGET, "Connection with peer {peer_id:?} not found in storage. Ignoring.");
                return;
            }
        };
        debug_assert!(peer_details.connection_id == connection.1);

        let old_state = peer_details.negotiated_state;
        peer_details.negotiated_state = NegotiatedPeerState::Unhealthy;

        if old_state != NegotiatedPeerState::Unhealthy {
            tracing::debug!(target: LOG_TARGET, "Peer {:?} has been marked as unhealthy.", connection.0);
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::UnhealthyPeer(connection.0)));
            self.try_wake();
        }
    }

    fn handle_healthy_connection(&mut self, connection: (PeerId, ConnectionId)) {
        let Some(peer_details) = self.negotiated_peers.get_mut(&connection.0) else {
            #[cfg(debug_assertions)]
            panic!(
                "Connection with peer {:?} not found in storage.",
                connection.0
            );
            #[cfg(not(debug_assertions))]
            {
                tracing::warn!(target: LOG_TARGET, "Connection with peer {peer_id:?} not found in storage. Ignoring.");
                return;
            }
        };
        debug_assert!(peer_details.connection_id == connection.1);

        let old_state = peer_details.negotiated_state;
        peer_details.negotiated_state = NegotiatedPeerState::Healthy;

        if old_state != NegotiatedPeerState::Healthy {
            tracing::debug!(target: LOG_TARGET, "Peer {:?} has been marked as healthy.", connection.0);
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(connection.0)));
            self.try_wake();
        }
    }

    fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        from: (PeerId, ConnectionId),
    ) {
        // Mark a peer as malicious if it sends a un-deserializable message: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df8172927bebb75d8b988e.
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            tracing::debug!(target: LOG_TARGET, "Failed to deserialize encapsulated message.");
            self.mark_connection_as_malicious(from);
            return;
        };

        let message_identifier = deserialized_encapsulated_message.id();

        // Mark a core peer as malicious if it sends a duplicate message maliciously (i.e., if a message with the same identifier was already exchanged with them): https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81fc86bdce264466efd3.
        let Ok(()) = self.check_and_update_message_cache(&message_identifier, from) else {
            return;
        };
        // Verify the message public header, or else mark the peer as malicious: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81859cebf5e3d2a5cd8f.
        let Ok(validated_message) = deserialized_encapsulated_message.validate_public_header()
        else {
            tracing::debug!(target: LOG_TARGET, "Neighbor sent us a message with an invalid public header. Marking it as spammy.");
            self.mark_connection_as_malicious(from);
            return;
        };

        // Notify the swarm about the received message, so that it can be further
        // processed by the core protocol module.
        self.events.push_back(ToSwarm::GenerateEvent(Event::Message(
            Box::new(validated_message),
            from.0,
        )));
        self.try_wake();
    }

    fn check_and_update_message_cache(
        &mut self,
        message_id: &MessageIdentifier,
        connection: (PeerId, ConnectionId),
    ) -> Result<(), ()> {
        let exchanged_message_identifiers = self
            .exchanged_message_identifiers
            .entry(connection.0)
            .or_default();
        if !exchanged_message_identifiers.insert(*message_id) {
            tracing::debug!(target: LOG_TARGET, "Neighbor {:?} on connection {:?} sent us a message previously already exchanged. Marking it as spammy.", connection.0, connection.1);
            self.mark_connection_as_malicious(connection);
            return Err(());
        }
        Ok(())
    }
}

impl<ObservationWindowClockProvider> NetworkBehaviour for Behaviour<ObservationWindowClockProvider>
where
    ObservationWindowClockProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    type ConnectionHandler = Either<
        ConnectionHandler<ObservationWindowClockProvider::IntervalStream>,
        DummyConnectionHandler,
    >;
    type ToSwarm = Event;

    #[expect(
        clippy::cognitive_complexity,
        reason = "It's good to keep everything in a single function here."
    )]
    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If the new peer makes the set of established connections too large, do not
        // try to upgrade the connection.
        if self.negotiated_peers.len() >= *self.peering_degree.end() {
            tracing::trace!(target: LOG_TARGET, "Inbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since we are already at maximum peering capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            tracing::debug!(target: LOG_TARGET, "Upgrading inbound connection {connection_id:?} with core peer {peer_id:?}.");
            return Ok(Either::Left(ConnectionHandler::new(
                ConnectionMonitor::new(self.observation_window_clock_provider.interval_stream()),
            )));
        };

        Ok(if membership.contains_remote(&peer_id) {
            tracing::debug!(target: LOG_TARGET, "Upgrading inbound connection {connection_id:?} with core peer {peer_id:?}.");
            Either::Left(ConnectionHandler::new(ConnectionMonitor::new(
                self.observation_window_clock_provider.interval_stream(),
            )))
        } else {
            tracing::debug!(target: LOG_TARGET, "Denying inbound connection {connection_id:?} with edge peer {peer_id:?}.");
            Either::Right(DummyConnectionHandler)
        })
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "It's good to keep everything in a single function here."
    )]
    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If the new peer makes the set of established connections too large, do not
        // try to upgrade the connection.
        if self.negotiated_peers.len() >= *self.peering_degree.end() {
            tracing::trace!(target: LOG_TARGET, "Outbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since we are already at maximum peering capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            tracing::debug!(target: LOG_TARGET, "Upgrading outbound connection {connection_id:?} with core peer {peer_id:?}.");
            return Ok(Either::Left(ConnectionHandler::new(
                ConnectionMonitor::new(self.observation_window_clock_provider.interval_stream()),
            )));
        };

        Ok(if membership.contains_remote(&peer_id) {
            tracing::debug!(target: LOG_TARGET, "Upgrading outbound connection {connection_id:?} with core peer {peer_id:?}.");
            Either::Left(ConnectionHandler::new(ConnectionMonitor::new(
                self.observation_window_clock_provider.interval_stream(),
            )))
        } else {
            tracing::debug!(target: LOG_TARGET, "Denying outbound connection {connection_id:?} with edge peer {peer_id:?}.");
            Either::Right(DummyConnectionHandler)
        })
    }

    /// Informs the behaviour about an event from the [`Swarm`].
    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed {
            peer_id,
            connection_id,
            ..
        }) = event
        {
            // This event happens in one of the following cases:
            // 1. The connection was closed by the peer.
            // 2. The connection was closed by the local node since no stream is active.
            let Some(peer_details) = self.negotiated_peers.remove(&peer_id) else {
                // This event was not meant for us to consume.
                return;
            };

            debug_assert!(peer_details.connection_id == connection_id);

            self.events
                .push_back(ToSwarm::GenerateEvent(Event::PeerDisconnected(
                    peer_id,
                    peer_details.negotiated_state,
                )));
            self.try_wake();
        }
    }

    /// Handles an event generated by the [`BlendConnectionHandler`]
    /// dedicated to the connection identified by `peer_id` and `connection_id`.
    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {
            Either::Left(event) => match event {
                // A message was forwarded from the peer.
                ToBehaviour::Message(message) => {
                    self.handle_received_serialized_encapsulated_message(
                        &message,
                        (peer_id, connection_id),
                    );
                }
                // The inbound/outbound connection was fully negotiated by the peer,
                // which means that the peer supports the blend protocol. We consider them healthy
                // by default.
                ToBehaviour::FullyNegotiatedInbound | ToBehaviour::FullyNegotiatedOutbound => {
                    self.handle_negotiated_connection((peer_id, connection_id));
                }
                ToBehaviour::SpammyPeer => {
                    self.handle_spammy_connection((peer_id, connection_id));
                }
                ToBehaviour::UnhealthyPeer => {
                    self.handle_unhealthy_connection((peer_id, connection_id));
                }
                ToBehaviour::HealthyPeer => {
                    self.handle_healthy_connection((peer_id, connection_id));
                }
                ToBehaviour::IOError(_) | ToBehaviour::DialUpgradeError(_) => {}
            },
        }
    }

    /// Polls for things that swarm should do.
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

pub trait IntervalStreamProvider {
    type IntervalStream: Stream<Item = Self::IntervalItem>;
    type IntervalItem;

    fn interval_stream(&self) -> Self::IntervalStream;
}
