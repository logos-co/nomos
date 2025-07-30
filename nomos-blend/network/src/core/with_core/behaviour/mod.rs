use std::{
    collections::{HashMap, VecDeque},
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

struct PeeringDegreeInfo {
    current_outgoing: usize,
    current_incoming: usize,
    min_outgoing: usize,
    max_total: usize,
}

impl PeeringDegreeInfo {
    const fn new(range: RangeInclusive<usize>) -> Self {
        Self {
            current_incoming: 0,
            current_outgoing: 0,
            min_outgoing: *range.start(),
            max_total: *range.end(),
        }
    }

    fn remaining_incoming_capacity(&self) -> usize {
        let max_incoming = self.max_total.saturating_sub(self.min_outgoing);
        let remaining_by_incoming = max_incoming.saturating_sub(self.current_incoming);
        let remaining_by_total = self
            .max_total
            .saturating_sub(self.current_outgoing + self.current_incoming);
        remaining_by_incoming.min(remaining_by_total)
    }

    fn remaining_outgoing_capacity(&self) -> usize {
        let available_slots = self
            .max_total
            .saturating_sub(self.current_incoming + self.current_outgoing);
        let required_to_reach_min = self.min_outgoing.saturating_sub(self.current_outgoing);
        available_slots.max(required_to_reach_min)
    }
}

/// A [`NetworkBehaviour`]:
/// - forwards messages to all connected core peers with deduplication.
/// - receives messages from all connected core peers.
pub struct Behaviour<ObservationWindowClockProvider> {
    negotiated_peers: HashMap<PeerId, (NegotiatedPeerState, Endpoint)>,
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
    peering_degree_info: PeeringDegreeInfo,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
enum NegotiatedPeerState {
    Healthy,
    Unhealthy,
}

#[derive(Debug)]
pub struct Config {
    pub seen_message_cache_size: usize,
    // Range representing (minimum outgoing peering degree, maximum incoming+outgoing peering
    // degree).
    pub peering_degree: RangeInclusive<usize>,
}

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the peers.
    Message(Vec<u8>, PeerId),
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
    ) -> Self {
        let duplicate_cache = SizedCache::with_size(config.seen_message_cache_size);
        Self {
            negotiated_peers: HashMap::with_capacity(*config.peering_degree.end()),
            events: VecDeque::new(),
            waker: None,
            seen_message_cache: duplicate_cache,
            observation_window_clock_provider,
            current_membership,
            peering_degree_info: PeeringDegreeInfo::new(config.peering_degree.clone()),
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
            return Err(Error::Duplicate);
        }

        let mut num_peers = 0;
        self.negotiated_peers
            .iter()
            // Exclude from the list of candidate peers the provided peer (i.e., the sender of the
            // message we are forwarding).
            .filter(|(peer_id, _)| (excluded_peer != Some(**peer_id)))
            // Exclude from the list of candidate peers any peer that is not in a healthy state.
            .filter(|(_, (peer_state, _))| *peer_state == NegotiatedPeerState::Healthy)
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

    #[must_use]
    pub fn num_healthy_peers(&self) -> usize {
        self.negotiated_peers
            .values()
            .filter(|(state, _)| *state == NegotiatedPeerState::Healthy)
            .count()
    }

    #[must_use]
    pub fn num_connected_outgoing_peers(&self) -> usize {
        self.negotiated_peers
            .values()
            .filter(|(_, endpoint)| *endpoint == Endpoint::Listener)
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
    }

    fn insert_peer(
        &mut self,
        peer_id: PeerId,
        state: NegotiatedPeerState,
        peer_endpoint: Endpoint,
    ) {
        let old_peer_state = self
            .negotiated_peers
            .insert(peer_id, (state, peer_endpoint));
        match (old_peer_state, peer_endpoint) {
            (None, Endpoint::Dialer) => {
                self.peering_degree_info.current_incoming = self
                    .peering_degree_info
                    .current_incoming
                    .checked_add(1)
                    .unwrap();
            }
            (None, Endpoint::Listener) => {
                self.peering_degree_info.current_outgoing = self
                    .peering_degree_info
                    .current_outgoing
                    .checked_add(1)
                    .unwrap();
            }
            _ => {}
        }
    }

    fn update_peer_state(
        &mut self,
        peer_id: PeerId,
        new_state: NegotiatedPeerState,
    ) -> Option<NegotiatedPeerState> {
        let old_state = self.negotiated_peers.get(&peer_id).map(|(state, _)| *state);
        self.negotiated_peers
            .entry(peer_id)
            .and_modify(|(state, _)| *state = new_state);
        old_state
    }

    fn remove_peer(&mut self, peer_id: &PeerId) -> Option<(NegotiatedPeerState, Endpoint)> {
        let Some((peer_state, endpoint)) = self.negotiated_peers.remove(peer_id) else {
            return None;
        };
        match endpoint {
            Endpoint::Dialer => {
                self.peering_degree_info.current_incoming = self
                    .peering_degree_info
                    .current_incoming
                    .checked_sub(1)
                    .unwrap();
            }
            Endpoint::Listener => {
                self.peering_degree_info.current_outgoing = self
                    .peering_degree_info
                    .current_outgoing
                    .checked_sub(1)
                    .unwrap();
            }
        };
        Some((peer_state, endpoint))
    }

    fn can_accept_incoming_connection(&self) -> bool {
        self.peering_degree_info.remaining_incoming_capacity() > 0
    }

    fn can_open_outgoing_connection(&self) -> bool {
        self.peering_degree_info.remaining_outgoing_capacity() > 0
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
        _: ConnectionId,
        peer_id: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if !self.can_accept_incoming_connection() {
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
            // Do not deny connection, but do not allow for any sub-stream to be created
            // with a non-core node, in this behaviour.
            Either::Right(DummyConnectionHandler)
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
        if !self.can_open_outgoing_connection() {
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
            // Do not deny connection, but do not allow for any sub-stream to be created
            // with a non-core node, in this behaviour.
            Either::Right(DummyConnectionHandler)
        })
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
                self.remove_peer(&peer_id);
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
                    self.handle_received_message(message, peer_id);
                }
                // The inbound/outbound connection was fully negotiated by the peer,
                // which means that the peer supports the blend protocol.
                ToBehaviour::FullyNegotiatedInbound => {
                    self.insert_peer(peer_id, NegotiatedPeerState::Healthy, Endpoint::Dialer);
                }
                ToBehaviour::FullyNegotiatedOutbound => {
                    self.insert_peer(peer_id, NegotiatedPeerState::Healthy, Endpoint::Listener);
                }
                ToBehaviour::DialUpgradeError(_) => {
                    self.remove_peer(&peer_id);
                }
                ToBehaviour::SpammyPeer => {
                    // Notify swarm only if it's the first occurrence.
                    if self.remove_peer(&peer_id).is_some() {
                        tracing::debug!("Peer {:?} has been detected as spammy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::SpammyPeer(peer_id)));
                    }
                }
                ToBehaviour::UnhealthyPeer => {
                    // Notify swarm only if it's the first transition into the unhealthy state.
                    let previous_state =
                        self.update_peer_state(peer_id, NegotiatedPeerState::Unhealthy);
                    if matches!(previous_state, None | Some(NegotiatedPeerState::Healthy)) {
                        tracing::debug!("Peer {:?} has been detected as unhealthy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::UnhealthyPeer(peer_id)));
                    }
                }
                ToBehaviour::HealthyPeer => {
                    // Notify swarm only if it's the first transition into the healthy state.
                    let previous_state =
                        self.update_peer_state(peer_id, NegotiatedPeerState::Healthy);
                    if matches!(previous_state, None | Some(NegotiatedPeerState::Unhealthy)) {
                        tracing::debug!("Peer {:?} has been detected as healthy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(peer_id)));
                    }
                }
                ToBehaviour::IOError(error) => {
                    self.remove_peer(&peer_id);
                    self.events.push_back(ToSwarm::GenerateEvent(Event::Error(
                        Error::PeerIOError {
                            error,
                            peer_id,
                            connection_id,
                        },
                    )));
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
