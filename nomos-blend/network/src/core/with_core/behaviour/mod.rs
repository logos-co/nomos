use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    convert::Infallible,
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
};

use cached::{Cached as _, SizedCache};
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
use nomos_blend_scheduling::membership::Membership;
use sha2::{Digest as _, Sha256};

use crate::core::with_core::{
    behaviour::handler::{
        conn_maintenance::ConnectionMonitor, ConnectionHandler, FromBehaviour, ToBehaviour,
    },
    error::Error,
};

mod handler;

#[cfg(feature = "tokio")]
pub use self::handler::tokio::ObservationWindowTokioIntervalProvider;

const LOG_TARGET: &str = "blend::network::core::core::behaviour";

/// A [`NetworkBehaviour`]:
/// - forwards messages to all connected core peers with deduplication.
/// - receives messages from all connected core peers.
pub struct Behaviour<ObservationWindowClockProvider> {
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    /// An LRU cache for storing seen messages (based on their ID). This
    /// cache prevents duplicates from being propagated on the network.
    // TODO: Once having the new message encapsulation mechanism,
    //       this cache should be <(key, nullifier), HashSet<PeerId>>.
    // TODO: This cache should be cleared after the session transition period has passed,
    //       because keys and nullifiers are valid during a single session.
    seen_message_cache: SizedCache<Vec<u8>, ()>,
    observation_window_clock_provider: ObservationWindowClockProvider,
    // TODO: Replace with the session stream and make this a non-Option
    current_membership: Option<Membership<PeerId>>,
    /// Peers that have established an inbound connection with us.
    ///
    /// If a peer support Blend and there are available incoming connection
    /// slots, the peer will be upgraded to a `Healthy` state once the
    /// connection handler is successfully initialized for the new connection.
    connected_incoming_peers: HashMap<(PeerId, ConnectionId), Option<NegotiatedPeerState>>,
    /// Peers that this peer has established an outgoing connection with.
    ///
    /// If a peer support Blend and there are available outgoing connection
    /// slots, the peer will be upgraded to a `Healthy` state once the
    /// connection handler is successfully initialized for the new connection.
    connected_outgoing_peers: HashMap<(PeerId, ConnectionId), Option<NegotiatedPeerState>>,
    /// Minimum outgoing peering degree, as per the Blend spec.
    min_outgoing: usize,
    /// Total number of incoming + outgoing connections the node can maintain at
    /// any given time.
    max_total: usize,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum NegotiatedPeerState {
    Healthy,
    Unhealthy,
    Spammy,
}

#[derive(Debug)]
pub struct Config {
    pub seen_message_cache_size: usize,
    // Range representing [minimum outgoing peering degree, maximum incoming+outgoing peering
    // degree].
    pub peering_degree: RangeInclusive<usize>,
}

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the peers.
    Message(Vec<u8>, PeerId),
    /// A peer on a given connection has been detected as unhealthy.
    UnhealthyPeer(PeerId, ConnectionId),
    /// A peer on a given connection that was previously unhealthy has returned
    /// to a healthy state.
    HealthyPeer(PeerId, ConnectionId),
    /// A connection with a peer has dropped. The last state that was negotiated
    /// with the peer is also returned.
    PeerDisconnected(PeerId, ConnectionId, NegotiatedPeerState),
    #[cfg(test)]
    /// For tests only. It signals to the swarm that a given incoming connection
    /// request from a peer was discarded.
    IncomingConnectionRequestDiscarded(PeerId, ConnectionId),
    #[cfg(test)]
    /// For tests only. It signals to the swarm that a given outgoing connection
    /// request to a peer was discarded.
    OutgoingConnectionRequestDiscarded(PeerId, ConnectionId),
    #[cfg(test)]
    /// For tests only. It signals to the swarm that a given incoming connection
    /// request from a peer was accepted and upgraded.
    IncomingConnectionRequestAccepted(PeerId, ConnectionId),
    #[cfg(test)]
    /// For tests only. It signals to the swarm that a given outgoing connection
    /// request to a peer was accepted and upgraded.
    OutgoingConnectionRequestAccepted(PeerId, ConnectionId),
}

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider> {
    #[must_use]
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
    ) -> Self {
        let duplicate_cache = SizedCache::with_size(config.seen_message_cache_size);
        let (min_outgoing, max_total) =
            (*config.peering_degree.start(), *config.peering_degree.end());
        let maximum_incoming = max_total.saturating_sub(min_outgoing);
        Self {
            events: VecDeque::new(),
            waker: None,
            seen_message_cache: duplicate_cache,
            observation_window_clock_provider,
            current_membership,
            max_total,
            min_outgoing,
            connected_incoming_peers: HashMap::with_capacity(maximum_incoming),
            connected_outgoing_peers: HashMap::with_capacity(max_total),
        }
    }

    /// Returns the number of slots that can be used for incoming connections.
    ///
    /// That is, the minimum between the maximum number of allowed incoming
    /// connections, and the number of remaining total slots.
    fn available_incoming_connections_slots(&self) -> usize {
        let max_incoming_slots = self.max_total.saturating_sub(self.min_outgoing);
        let remaining_slots = self
            .max_total
            .saturating_sub(self.connected_incoming_peers.len())
            .saturating_sub(self.connected_outgoing_peers.len());

        max_incoming_slots.min(remaining_slots)
    }

    /// Returns `true` if this node can try to upgrade an incoming connection
    /// request.
    fn can_accept_incoming_connection(&self) -> bool {
        self.available_incoming_connections_slots() > 0
    }

    /// Returns the number of slots that can be used for outgoing connections.
    ///
    /// Outgoing connections are allocated a minimum number of slots, plus, if
    /// needed, the remaining number of incoming slots.
    fn available_outgoing_connections_slots(&self) -> usize {
        self.max_total
            .saturating_sub(self.connected_incoming_peers.len())
            .saturating_sub(self.connected_outgoing_peers.len())
    }

    /// Returns `true` if this node can try to upgrade an incoming connection
    /// request.
    pub fn can_open_outgoing_connection(&self) -> bool {
        self.available_outgoing_connections_slots() > 0
    }

    /// Remove the connection from the internal storage and returns the role of
    /// the removed peer (i.e., dialer or listener) along with their latest
    /// negotiated state, if present.
    fn remove_connection(
        &mut self,
        connection: &(PeerId, ConnectionId),
    ) -> Option<(Endpoint, Option<NegotiatedPeerState>)> {
        if let Some(negotiated_state) = self.connected_incoming_peers.remove(connection) {
            Some((Endpoint::Dialer, negotiated_state))
        } else {
            self.connected_outgoing_peers
                .remove(connection)
                .map(|negotiated_state| (Endpoint::Listener, negotiated_state))
        }
    }

    /// Update the negotiated state of a given incoming or outgoing connection,
    /// returning the previous state, if one was set.
    fn update_state(
        &mut self,
        connection: (PeerId, ConnectionId),
        new_state: NegotiatedPeerState,
    ) -> Option<NegotiatedPeerState> {
        if let Entry::Occupied(mut existing_incoming_entry) =
            self.connected_incoming_peers.entry(connection)
        {
            let old_state = *existing_incoming_entry.get();
            *existing_incoming_entry.get_mut() = Some(new_state);
            old_state
        } else if let Entry::Occupied(mut existing_outgoing_entry) =
            self.connected_outgoing_peers.entry(connection)
        {
            let old_state = *existing_outgoing_entry.get();
            *existing_outgoing_entry.get_mut() = Some(new_state);
            old_state
        } else {
            None
        }
    }

    /// Publish an unseen message to all connected peers
    pub fn publish(&mut self, message: &[u8]) -> Result<(), Error> {
        self.forward_message_and_maybe_exclude(message, None)
    }

    fn forward_message_and_maybe_exclude(
        &mut self,
        message: &[u8],
        excluded_peer: Option<PeerId>,
    ) -> Result<(), Error> {
        let msg_id = Self::message_id(message);
        // If the message was already seen, don't forward it again
        if self.seen_message_cache.cache_get(&msg_id).is_some() {
            return Err(Error::DuplicateMessage);
        }

        let mut num_peers = 0;
        self.connected_incoming_peers
            .iter()
            .chain(self.connected_outgoing_peers.iter())
            .filter(|(_, state)| matches!(state, Some(NegotiatedPeerState::Healthy)))
            .map(|(peer_id, _)| peer_id)
            // Exclude from the list of candidate peers the provided peer (i.e., the sender of the
            // message we are forwarding).
            .filter(|(peer_id, _)| (excluded_peer != Some(*peer_id)))
            .for_each(|(peer_id, connection_id)| {
                tracing::debug!(target: LOG_TARGET, "Registering event for peer {peer_id:?} on connection {connection_id:?} to send msg");
                self.events.push_back(ToSwarm::NotifyHandler {
                    peer_id: *peer_id,
                    handler: NotifyHandler::One(*connection_id),
                    event: Either::Left(FromBehaviour::Message(message.to_vec())),
                });
                num_peers += 1;
            });

        if num_peers == 0 {
            Err(Error::NoPeers)
        } else {
            self.seen_message_cache.cache_set(msg_id, ());
            self.try_wake();
            Ok(())
        }
    }

    /// Forwards a message to all connected and healthy peers except the
    /// excluded peer.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers that support
    /// the blend protocol.
    pub fn forward_message(&mut self, message: &[u8], excluded_peer: PeerId) -> Result<(), Error> {
        self.forward_message_and_maybe_exclude(message, Some(excluded_peer))
    }

    fn message_id(message: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(message);
        hasher.finalize().to_vec()
    }

    /// Return the number of Blend peers on both incoming and outgoing
    /// connections that are marked as healthy by this node.
    #[must_use]
    pub fn num_healthy_peers(&self) -> usize {
        self.connected_incoming_peers
            .iter()
            .chain(self.connected_outgoing_peers.iter())
            .filter(|(_, state)| matches!(state, Some(NegotiatedPeerState::Healthy)))
            .count()
    }

    /// Return the number of outgoing connections to other Blend nodes that are
    /// marked as healthy by this node.
    pub fn healthy_outgoing_connections(&self) -> usize {
        self.connected_outgoing_peers
            .iter()
            .filter(|(_, state)| matches!(state, Some(NegotiatedPeerState::Healthy)))
            .count()
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn handle_received_message(&mut self, message: Vec<u8>, from: PeerId) {
        // If the message was already seen, ignore it.
        if self
            .seen_message_cache
            .cache_get(&Self::message_id(&message))
            .is_some()
        {
            return;
        }

        // Notify the swarm about the received message,
        // so that it can be processed by the core protocol module.
        self.events
            .push_back(ToSwarm::GenerateEvent(Event::Message(message, from)));
        self.try_wake();
    }

    fn handle_stream_upgrade(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        let old_state = self.update_state((peer_id, connection_id), NegotiatedPeerState::Healthy);
        #[cfg(test)]
        {
            // Do not process the second channel upgrade.
            if old_state == Some(NegotiatedPeerState::Healthy) {
                return;
            }

            let is_peer_outgoing = self
                .connected_outgoing_peers
                .contains_key(&(peer_id, connection_id));
            let is_peer_incoming = self
                .connected_incoming_peers
                .contains_key(&(peer_id, connection_id));

            if is_peer_outgoing {
                self.events.push_back(ToSwarm::GenerateEvent(
                    Event::OutgoingConnectionRequestAccepted(peer_id, connection_id),
                ));
                self.try_wake();
            } else if is_peer_incoming {
                self.events.push_back(ToSwarm::GenerateEvent(
                    Event::IncomingConnectionRequestAccepted(peer_id, connection_id),
                ));
                self.try_wake();
            }
        }
        // Keep Clippy happy
        let _ = old_state;
    }

    fn handle_spammy_peer(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        // We do not remove the peer yet, as that will happen once the connection is
        // closed and we capture the respective swarm event. This will happen for sure
        // since the connection handler will not deal with the peer anymore.
        // Also, we do not notify the swarm about a spammy peer, because it might try to
        // open a new connection, but until the connection to the spammy peer is not
        // completely closed, the new outgoing connection might fail due to connection
        // limits (which is calculated from number of elements in our storage maps). So,
        // we let the swarm know about it when the peer has actually
        // disconnected and removed from storage.
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id:?} on connection {connection_id:?} has been detected as spammy");
        self.update_state((peer_id, connection_id), NegotiatedPeerState::Spammy);
    }

    fn handle_unhealthy_peer(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        // Notify swarm only if it's the first transition from a previously negotiated
        // state into the unhealthy state.
        let Some(old_state) =
            self.update_state((peer_id, connection_id), NegotiatedPeerState::Unhealthy)
        else {
            return;
        };
        if old_state != NegotiatedPeerState::Unhealthy {
            tracing::debug!(target: LOG_TARGET, "Peer {:?} on connection {connection_id:?} has been detected as unhealthy", peer_id);
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::UnhealthyPeer(
                    peer_id,
                    connection_id,
                )));
            self.try_wake();
        }
    }

    fn handle_healthy_peer(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        // Notify swarm only if it's the first transition from a previously negotiated
        // state into the healthy state.
        let Some(old_state) =
            self.update_state((peer_id, connection_id), NegotiatedPeerState::Healthy)
        else {
            return;
        };
        if old_state != NegotiatedPeerState::Healthy {
            tracing::debug!(target: LOG_TARGET, "Peer {:?} on connection {connection_id:?} has been detected as healthy", peer_id);
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(
                    peer_id,
                    connection_id,
                )));
            self.try_wake();
        }
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

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // We calculate this before adding the peer to the storage, so we know whether
        // we have a slot available to accept a new incoming connection or not.
        let can_accept_incoming_connection = self.can_accept_incoming_connection();
        // Add connection regardless of the handler we will use, to enforce a limit on
        // the connection requests.
        self.connected_incoming_peers
            .insert((peer_id, connection_id), None);

        // If we are above the maximum limit, return a dummy handler that will soon
        // close the connection, if the swarm is configured to do so.
        if !can_accept_incoming_connection {
            tracing::trace!(target: LOG_TARGET, "Connected peer {peer_id:?} on connection {connection_id:?} will not be upgraded since we are already at maximum incoming connection capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            return Ok(Either::Left(ConnectionHandler::new(
                ConnectionMonitor::new(self.observation_window_clock_provider.interval_stream()),
            )));
        };

        Ok(if membership.contains_remote(&peer_id) {
            Either::Left(ConnectionHandler::new(ConnectionMonitor::new(
                self.observation_window_clock_provider.interval_stream(),
            )))
        } else {
            Either::Right(DummyConnectionHandler)
        })
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // We calculate this before adding the peer to the storage, so we know whether
        // we have a slot available to open a new outgoing connection or not.
        let can_open_outgoing_connection = self.can_open_outgoing_connection();
        // Add connection regardless of the handler we will use, to enforce a limit on
        // the connection requests.
        self.connected_outgoing_peers
            .insert((peer_id, connection_id), None);

        // If we are above the maximum limit, return a dummy handler that will soon
        // close the connection, if the swarm is configured to do so.
        if !can_open_outgoing_connection {
            tracing::trace!(target: LOG_TARGET, "Connected peer {peer_id:?} on connection {connection_id:?} will not be upgraded since we are already at maximum outgoing connection capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            return Ok(Either::Left(ConnectionHandler::new(
                ConnectionMonitor::new(self.observation_window_clock_provider.interval_stream()),
            )));
        };

        Ok(if membership.contains_remote(&peer_id) {
            Either::Left(ConnectionHandler::new(ConnectionMonitor::new(
                self.observation_window_clock_provider.interval_stream(),
            )))
        } else {
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
            //
            // In both cases, we need to remove the peer from the list of connected peers.
            let Some((peer_role, negotiated_state)) =
                self.remove_connection(&(peer_id, connection_id))
            else {
                // This event is not for us to consume.
                return;
            };

            match (peer_role, negotiated_state) {
                // If the disconnected peer was part of an outgoing connection from us (i.e., it was
                // a listener) and we had a negotiated state with it (i.e., it was a Blend node),
                // notify the swarm about it.
                (Endpoint::Listener, Some(state)) => {
                    self.events
                        .push_back(ToSwarm::GenerateEvent(Event::PeerDisconnected(
                            peer_id,
                            connection_id,
                            state,
                        )));
                    self.try_wake();
                }
                // Disconnected incoming connections are not an issue the swarm must deal with, so
                // we keep it internal, i.e., we will just accept a new incoming connection when it
                // comes, if we have a slot available for it.
                (Endpoint::Dialer, _) => {}
                // The disconnected peer was never upgraded to the Blend
                // protocol (i.e., its negotiated state is `None`), either because they do not
                // support it or because we prevented it due to connection limits.
                // So, we do not notify the swarm at all about this, as this is an
                // internal detail.
                // We bubble this up to the swarm only during tests, to test that connections beyond
                // our maximum capacity are effectively ignored.
                (peer_role, None) => {
                    #[cfg(test)]
                    {
                        match peer_role {
                            Endpoint::Dialer => {
                                self.events.push_back(ToSwarm::GenerateEvent(
                                    Event::IncomingConnectionRequestDiscarded(
                                        peer_id,
                                        connection_id,
                                    ),
                                ));
                            }
                            Endpoint::Listener => {
                                self.events.push_back(ToSwarm::GenerateEvent(
                                    Event::OutgoingConnectionRequestDiscarded(
                                        peer_id,
                                        connection_id,
                                    ),
                                ));
                            }
                        }
                        self.try_wake();
                    };
                    // Make Clippy happy since `peer_role` is only used in a test-gated block.
                    let _ = peer_role;
                }
            }
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
                    self.handle_received_message(message, peer_id);
                }
                // The inbound/outbound connection was fully negotiated by the peer,
                // which means that the peer supports the blend protocol.
                ToBehaviour::FullyNegotiatedInbound | ToBehaviour::FullyNegotiatedOutbound => {
                    self.handle_stream_upgrade(peer_id, connection_id);
                }
                ToBehaviour::SpammyPeer => self.handle_spammy_peer(peer_id, connection_id),
                ToBehaviour::UnhealthyPeer => {
                    self.handle_unhealthy_peer(peer_id, connection_id);
                }
                ToBehaviour::HealthyPeer => {
                    self.handle_healthy_peer(peer_id, connection_id);
                }
                // We do not do anything with these errors, since they will result in the connection
                // with the peer closed, which we capture and process elsewhere, and we also notify
                // the swarm about it then, to avoid concurrency issues in the number of connections
                // that are kept open.
                // TODO: These two events can then be removed from the connection handler.
                ToBehaviour::DialUpgradeError(_) | ToBehaviour::IOError(_) => {}
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
