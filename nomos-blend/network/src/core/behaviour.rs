use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
    time::Duration,
};

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
use nomos_blend_message::MessageIdentifier;
use nomos_blend_scheduling::{
    deserialize_encapsulated_message, membership::Membership,
    message_blend::crypto::CryptographicProcessor, serialize_encapsulated_message,
    EncapsulatedMessage, UnwrappedMessage,
};

use crate::{
    core::{
        conn_maintenance::ConnectionMonitor,
        handler::{
            core::{self, FromBehaviour, ToBehaviour},
            edge,
        },
        Error,
    },
    message::{EncapsulatedMessageWithValidatedPublicHeader, ValidateMessagePublicHeader as _},
};

const LOG_TARGET: &str = "blend::network::behaviour";

/// A [`NetworkBehaviour`] that processes incoming Blend messages, and
/// propagates messages from the Blend service to the rest of the Blend network.
///
/// The public header and uniqueness of incoming messages is validated according to the [Blend v1 specification](https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084) before the message is propagated to the swarm and to the Blend service.
/// The same checks are applied to messages received by the Blend service before
/// they are propagated to the rest of the network, making sure no peer marks
/// this node as malicious due to an invalid Blend message.
pub struct Behaviour<Rng, ObservationWindowClockProvider> {
    negotiated_peers: HashMap<PeerId, NegotiatedPeerState>,
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, ()>>>,
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
    edge_node_connection_duration: Duration,
    cryptographic_processor: CryptographicProcessor<PeerId, Rng>,
}

#[derive(Debug, Eq, PartialEq)]
enum NegotiatedPeerState {
    Healthy,
    Unhealthy,
}

pub enum Event {
    /// A message received from one of the peers, after its correctness has been
    /// verified by the cryptographic processor.
    Message(Box<UnwrappedMessage>),
    /// A peer has been detected as spammy.
    SpammyPeer(PeerId),
    /// A peer has been detected as unhealthy.
    UnhealthyPeer(PeerId),
    /// A peer has been detected as healthy.
    HealthyPeer(PeerId),
    Error(Error),
}

/// The source of a Blend message.
///
/// Used to avoid received messages are sent back to the sender, in case the
/// sender is a core node.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PeerSource {
    /// The sender was a core node with the given `PeerId`.
    Core(PeerId),
    /// The sender was an edge node.
    Edge,
}

impl PeerSource {
    const fn sender_address(&self) -> Option<PeerId> {
        match self {
            Self::Core(peer_id) => Some(*peer_id),
            Self::Edge => None,
        }
    }
}

impl From<PeerId> for PeerSource {
    fn from(peer_id: PeerId) -> Self {
        Self::Core(peer_id)
    }
}

impl<Rng, ObservationWindowClockProvider> Behaviour<Rng, ObservationWindowClockProvider> {
    #[must_use]
    pub fn new(
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
        edge_node_connection_duration: Duration,
        cryptographic_processor: CryptographicProcessor<PeerId, Rng>,
    ) -> Self {
        Self {
            negotiated_peers: HashMap::new(),
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
            edge_node_connection_duration,
            cryptographic_processor,
        }
    }

    /// Publish an already-encapsulated message to all connected peers.
    ///
    /// Before the message is propagated, its public header is validated to make
    /// sure the receiving peer won't mark us as malicious.
    pub fn validate_and_publish(&mut self, message: EncapsulatedMessage) -> Result<(), Error> {
        let validated_message = message
            .validate_public_header()
            .map_err(|_| Error::InvalidMessage)?;
        self.forward_message(&validated_message, None)?;
        self.try_wake();
        Ok(())
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
    fn forward_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        excluded_peer: Option<PeerId>,
    ) -> Result<(), Error> {
        let message_id = message.id();

        let serialized_message = serialize_encapsulated_message(message);
        let mut num_peers = 0;
        self.negotiated_peers
            .iter()
            // Exclude from the list of candidate peers the provided peer (i.e., the sender of the
            // message we are forwarding).
            .filter(|(peer_id, _)| (excluded_peer != Some(**peer_id)))
            // Exclude from the list of candidate peers any peer that is not in a healthy state.
            .filter(|(_, peer_state)| **peer_state == NegotiatedPeerState::Healthy)
            .for_each(|(peer_id, _)| {
                if self
                    .exchanged_message_identifiers
                    .entry(*peer_id)
                    .or_default()
                    .insert(message_id)
                {
                    tracing::debug!(target: LOG_TARGET, "Registering event for peer {:?} to send msg", peer_id);
                    self.events.push_back(ToSwarm::NotifyHandler {
                        peer_id: *peer_id,
                        handler: NotifyHandler::Any,
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
            Ok(())
        }
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

    /// Mark the sender of a malformed message as malicious if the sender is a
    /// core node.
    fn mark_peer_as_malicious(&mut self, peer_source: &PeerSource) {
        if let PeerSource::Core(peer_id) = peer_source {
            tracing::debug!(target: LOG_TARGET, "Closing substream and marking core peer as malicious {peer_id:?}.");
            self.close_spammy_substream(*peer_id);
        }
    }

    /// Remove the peer from the set of negotiated peers and instruct the swarm
    /// to close the Blend substream with the specified peer.
    ///
    /// This method also cleans up the history of messages exchanged with such
    /// peer.
    fn close_spammy_substream(&mut self, peer_id: PeerId) {
        // Notify swarm only if it's the first occurrence.
        if self.negotiated_peers.remove(&peer_id).is_some() {
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::SpammyPeer(peer_id)));
        }
        // Clear the cache.
        self.exchanged_message_identifiers.remove(&peer_id);
    }

    fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        source: &PeerSource,
    ) {
        // Mark a peer as malicious if it sends a un-deserializable message: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df8172927bebb75d8b988e.
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            tracing::debug!(target: LOG_TARGET, "Failed to deserialize encapsulated message.");
            self.mark_peer_as_malicious(source);
            return;
        };

        let message_identifier = deserialized_encapsulated_message.id();

        // Mark a (core) peer as malicious if it sends a duplicate message maliciously (i.e., if a message with the same identifier was already exchanged with them): https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81fc86bdce264466efd3.
        if let PeerSource::Core(peer_id) = *source {
            let Ok(()) = self.check_and_update_message_cache(&message_identifier, peer_id) else {
                return;
            };
        }

        // Verify the message public header, or else mark the peer as malicious: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81859cebf5e3d2a5cd8f.
        let Ok(validated_message) = deserialized_encapsulated_message.validate_public_header()
        else {
            tracing::debug!(target: LOG_TARGET, "Neighbor sent us a message with an invalid public header. Marking it as spammy.");
            self.mark_peer_as_malicious(source);
            return;
        };

        // Forward the (un-decapsulated but validated) message immediately to the rest
        // of connected peers before any further processing for fast
        // propagation.
        if let Err(e) = self.forward_message(&validated_message, source.sender_address()) {
            tracing::error!(target: LOG_TARGET, "Failed to forward message: {e:?}");
        }

        // Start the processing.
        let Ok(decapsulated_message) = self.try_decapsulate_message(validated_message.into_inner())
        else {
            return;
        };

        // Notify the swarm about the received message,
        // so that it can be further processed by the core protocol module.
        self.events
            .push_back(ToSwarm::GenerateEvent(Event::Message(Box::new(
                decapsulated_message,
            ))));
    }

    fn check_and_update_message_cache(
        &mut self,
        message_id: &MessageIdentifier,
        peer_id: PeerId,
    ) -> Result<(), ()> {
        let exchanged_message_identifiers = self
            .exchanged_message_identifiers
            .entry(peer_id)
            .or_default();
        if !exchanged_message_identifiers.insert(*message_id) {
            tracing::debug!(target: LOG_TARGET, "Neighbor {peer_id:?} sent us a message previously already exchanged. Marking it as spammy.");
            self.mark_peer_as_malicious(&peer_id.into());
            return Err(());
        }
        Ok(())
    }

    fn try_decapsulate_message(
        &self,
        encapsulated_message: EncapsulatedMessage,
    ) -> Result<UnwrappedMessage, ()> {
        self.cryptographic_processor
            .decapsulate_message(encapsulated_message)
            .map_err(|e| {
                tracing::debug!(target: LOG_TARGET, "Failed to decapsulate message: {e:?}");
            })
    }
}

impl<Rng, ObservationWindowClockProvider> Behaviour<Rng, ObservationWindowClockProvider>
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

impl<Rng, ObservationWindowClockProvider> NetworkBehaviour
    for Behaviour<Rng, ObservationWindowClockProvider>
where
    Rng: 'static,
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
                    self.handle_received_serialized_encapsulated_message(
                        &message,
                        &PeerSource::Core(peer_id),
                    );
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
                    tracing::debug!(target: LOG_TARGET, "Peer {:?} has been detected as spammy", peer_id);
                    self.close_spammy_substream(peer_id);
                }
                ToBehaviour::UnhealthyPeer => {
                    // Notify swarm only if it's the first transition into the unhealthy state.
                    let previous_state = self
                        .negotiated_peers
                        .insert(peer_id, NegotiatedPeerState::Unhealthy);
                    if matches!(previous_state, None | Some(NegotiatedPeerState::Healthy)) {
                        tracing::debug!(target: LOG_TARGET, "Peer {:?} has been detected as unhealthy", peer_id);
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
                        tracing::debug!(target: LOG_TARGET, "Peer {:?} has been detected as healthy", peer_id);
                        self.events
                            .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(peer_id)));
                    }
                }
                ToBehaviour::IOError(error) => {
                    self.negotiated_peers.remove(&peer_id);
                    self.events
                        .push_back(ToSwarm::GenerateEvent(Event::Error(Error::PeerIO {
                            error,
                            peer_id,
                            connection_id,
                        })));
                }
            },
            Either::Right(event) => match event {
                // We "shuffle" together messages received from core and edge nodes.
                // The difference is that for messages received by edge nodes, we forward them to
                // all connected core nodes.
                edge::ToBehaviour::Message(new_message) => {
                    self.handle_received_serialized_encapsulated_message(
                        &new_message,
                        &PeerSource::Edge,
                    );
                }
                edge::ToBehaviour::FailedReception(reason) => {
                    tracing::trace!(target: LOG_TARGET, "An attempt was made from an edge node to send a message to us, but the attempt failed. Error reason: {reason:?}");
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
