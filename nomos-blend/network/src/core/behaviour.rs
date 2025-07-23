use std::{
    collections::{HashMap, VecDeque},
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
    time::Duration,
};

use cached::{Cached as _, SizedCache};
use either::Either;
use futures::Stream;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionClosed, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour,
        NotifyHandler, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_scheduling::membership::Membership;
use sha2::{Digest as _, Sha256};

use crate::core::{
    conn_maintenance::ConnectionMonitor,
    handler::{
        core::{self, FromBehaviour, ToBehaviour},
        edge,
    },
    Error,
};

/// A [`NetworkBehaviour`]:
/// - forwards messages to all connected peers with deduplication.
/// - receives messages from all connected peers.
pub struct Behaviour<ObservationWindowClockProvider> {
    negotiated_peers: HashMap<PeerId, NegotiatedPeerState>,
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, ()>>>,
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
    edge_node_connection_duration: Duration,
}

#[derive(Debug, Eq, PartialEq)]
enum NegotiatedPeerState {
    Healthy,
    Unhealthy,
}

#[derive(Debug)]
pub struct Config {
    pub seen_message_cache_size: usize,
}

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the peers.
    Message(Vec<u8>),
    /// A peer has been detected as spammy.
    SpammyPeer(PeerId),
    /// A peer has been detected as unhealthy.
    UnhealthyPeer(PeerId),
    /// A peer has been detected as healthy.
    HealthyPeer(PeerId),
    Error(Error),
}

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider> {
    #[must_use]
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
        edge_node_connection_duration: Duration,
    ) -> Self {
        let duplicate_cache = SizedCache::with_size(config.seen_message_cache_size);
        Self {
            negotiated_peers: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            seen_message_cache: duplicate_cache,
            observation_window_clock_provider,
            current_membership,
            edge_node_connection_duration,
        }
    }

    /// Publish a message to all connected peers
    pub fn publish(&mut self, message: &[u8]) -> Result<(), Error> {
        let msg_id = Self::message_id(message);
        // If the message was already seen, don't forward it again
        if self.seen_message_cache.cache_get(&msg_id).is_some() {
            return Ok(());
        }

        let result = self.forward_message(message, None);
        // Add the message to the cache only if the forwarding was successfully
        // triggered
        if result.is_ok() {
            self.seen_message_cache.cache_set(msg_id, ());
        }
        result
    }

    /// Forwards a message to all connected and healthy peers except the
    /// excluded peer.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers that support
    /// the blend protocol.
    fn forward_message(
        &mut self,
        message: &[u8],
        excluded_peer: Option<PeerId>,
    ) -> Result<(), Error> {
        let mut num_peers = 0;
        self.negotiated_peers
            .iter()
            // Exclude from the list of candidate peers the provided peer (i.e., the sender of the
            // message we are forwarding).
            .filter(|(peer_id, _)| (excluded_peer != Some(**peer_id)))
            // Exclude from the list of candidate peers any peer that is not in a healthy state.
            .filter(|(_, peer_state)| **peer_state == NegotiatedPeerState::Healthy)
            .for_each(|(peer_id, _)| {
                tracing::debug!("Registering event for peer {:?} to send msg", peer_id);
                self.events.push_back(ToSwarm::NotifyHandler {
                    peer_id: *peer_id,
                    handler: NotifyHandler::Any,
                    event: Either::Left(FromBehaviour::Message(message.to_vec())),
                });
                num_peers += 1;
            });

        if num_peers == 0 {
            Err(Error::NoPeers)
        } else {
            self.try_wake();
            Ok(())
        }
    }

    fn message_id(message: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(message);
        hasher.finalize().to_vec()
    }

    #[must_use]
    pub fn num_healthy_peers(&self) -> usize {
        self.negotiated_peers
            .values()
            .filter(|state| **state == NegotiatedPeerState::Healthy)
            .count()
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn handle_received_message(&mut self, message: Vec<u8>, from: Option<PeerId>) {
        // Add the message to the cache. If it was already seen, ignore it.
        if self
            .seen_message_cache
            .cache_set(Self::message_id(&message), ())
            .is_some()
        {
            return;
        }

        // Forward the message immediately to the rest of connected peers
        // without any processing for the fast propagation.
        if let Err(e) = self.forward_message(&message, from) {
            tracing::error!("Failed to forward message: {e:?}");
        }

        // Notify the swarm about the received message,
        // so that it can be processed by the core protocol module.
        self.events
            .push_back(ToSwarm::GenerateEvent(Event::Message(message)));
    }
}

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider>
where
    ObservationWindowClockProvider: IntervalStreamProvider<IntervalItem = RangeInclusive<u64>>,
{
    fn create_connection_handler_for_remote_core(
        &self,
    ) -> core::ConnectionHandler<ObservationWindowClockProvider::IntervalStream> {
        core::ConnectionHandler::new(ConnectionMonitor::new(
            self.observation_window_clock_provider.interval_stream(),
        ))
    }

    fn create_connection_handler_for_remote_edge(&self) -> edge::ConnectionHandler {
        edge::ConnectionHandler::new(self.edge_node_connection_duration)
    }
}

impl<ObservationWindowClockProvider> NetworkBehaviour for Behaviour<ObservationWindowClockProvider>
where
    ObservationWindowClockProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    type ConnectionHandler = Either<
        core::ConnectionHandler<ObservationWindowClockProvider::IntervalStream>,
        edge::ConnectionHandler,
    >;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            return Ok(Either::Left(
                self.create_connection_handler_for_remote_core(),
            ));
        };
        Ok(if membership.contains_remote(&peer_id) {
            Either::Left(self.create_connection_handler_for_remote_core())
        } else {
            Either::Right(self.create_connection_handler_for_remote_edge())
        })
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If no membership is provided (for tests), then we assume all peers are core
        // nodes.
        let Some(membership) = &self.current_membership else {
            return Ok(Either::Left(
                self.create_connection_handler_for_remote_core(),
            ));
        };
        if membership.contains_remote(&peer_id) {
            Ok(Either::Left(
                self.create_connection_handler_for_remote_core(),
            ))
        } else {
            Err(ConnectionDenied::new(
                "No outbound stream is expected toward edge nodes.",
            ))
        }
    }

    /// Informs the behaviour about an event from the [`Swarm`].
    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed {
            peer_id,
            remaining_established,
            ..
        }) = event
        {
            // This event happens in one of the following cases:
            // 1. The connection was closed by the peer.
            // 2. The connection was closed by the local node since no stream is active.
            //
            // In both cases, we need to remove the peer from the list of connected peers,
            // though it may be already removed from list by handling other events.
            if remaining_established == 0 {
                self.negotiated_peers.remove(&peer_id);
            }
        }

        self.try_wake();
    }

    /// Handles an event generated by the [`BlendConnectionHandler`]
    /// dedicated to the connection identified by `peer_id` and `connection_id`.
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
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
                    self.handle_received_message(message, Some(peer_id));
                }
                // The inbound/outbound connection was fully negotiated by the peer,
                // which means that the peer supports the blend protocol.
                ToBehaviour::FullyNegotiatedInbound | ToBehaviour::FullyNegotiatedOutbound => {
                    self.negotiated_peers
                        .insert(peer_id, NegotiatedPeerState::Healthy);
                }
                ToBehaviour::DialUpgradeError(_) => {
                    self.negotiated_peers.remove(&peer_id);
                }
                ToBehaviour::SpammyPeer => {
                    // Notify swarm only if it's the first occurrence.
                    if self.negotiated_peers.remove(&peer_id).is_some() {
                        tracing::debug!("Peer {:?} has been detected as spammy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::SpammyPeer(peer_id)));
                    }
                }
                ToBehaviour::UnhealthyPeer => {
                    // Notify swarm only if it's the first transition into the unhealthy state.
                    let previous_state = self
                        .negotiated_peers
                        .insert(peer_id, NegotiatedPeerState::Unhealthy);
                    if matches!(previous_state, None | Some(NegotiatedPeerState::Healthy)) {
                        tracing::debug!("Peer {:?} has been detected as unhealthy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::UnhealthyPeer(peer_id)));
                    }
                }
                ToBehaviour::HealthyPeer => {
                    // Notify swarm only if it's the first transition into the healthy state.
                    let previous_state = self
                        .negotiated_peers
                        .insert(peer_id, NegotiatedPeerState::Healthy);
                    if matches!(previous_state, None | Some(NegotiatedPeerState::Unhealthy)) {
                        tracing::debug!("Peer {:?} has been detected as healthy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(peer_id)));
                    }
                }
                ToBehaviour::IOError(error) => {
                    self.negotiated_peers.remove(&peer_id);
                    self.events.push_back(ToSwarm::GenerateEvent(Event::Error(
                        Error::PeerIOError {
                            error,
                            peer_id,
                            connection_id,
                        },
                    )));
                }
            },
            Either::Right(event) => match event {
                // We "shuffle" together messages received from core and edge nodes.
                // The difference is that for messages received by edge nodes, we forward them to
                // all connected core nodes.
                edge::ToBehaviour::Message(new_message) => {
                    self.handle_received_message(new_message, None);
                }
                edge::ToBehaviour::FailedReception(reason) => {
                    tracing::trace!("An attempt was made from an edge node to send a message to us, but the attempt failed. Error reason: {reason:?}");
                }
            },
        }

        self.try_wake();
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
