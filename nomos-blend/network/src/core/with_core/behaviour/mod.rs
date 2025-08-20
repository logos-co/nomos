use std::{
    collections::VecDeque,
    convert::Infallible,
    ops::{Deref, DerefMut, RangeInclusive},
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
        behaviour::{
            handler::{
                conn_maintenance::ConnectionMonitor, ConnectionHandler, FromBehaviour, ToBehaviour,
            },
            session::{Session, SessionRef, Sessions},
        },
        error::Error,
    },
    current_session_behaviour,
    message::{EncapsulatedMessageWithValidatedPublicHeader, ValidateMessagePublicHeader as _},
    previous_session_behaviour,
};

mod handler;
mod macros;
mod session;

#[cfg(feature = "tokio")]
pub use self::handler::tokio::ObservationWindowTokioIntervalProvider;

const LOG_TARGET: &str = "blend::network::core::core::behaviour";

#[derive(Debug)]
pub struct Config {
    /// The [minimum, maximum] peering degree of this node.
    pub peering_degree: RangeInclusive<usize>,
}

#[derive(Debug, Clone, Copy)]
struct RemotePeerConnectionDetails {
    /// Role of the remote peer in this connection.
    role: Endpoint,
    /// Latest negotiated state of the peer.
    negotiated_state: NegotiatedPeerState,
    /// The ID of the connection with the peer.
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
    /// The current and the previous sessions
    sessions: Sessions,
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    observation_window_clock_provider: ObservationWindowClockProvider,
    /// The [minimum, maximum] peering degree of this node.
    peering_degree: RangeInclusive<usize>,
    local_peer_id: PeerId,
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
    Message(
        Box<EncapsulatedMessageWithValidatedPublicHeader>,
        (PeerId, ConnectionId),
    ),
    /// A peer on a given connection has been detected as unhealthy.
    UnhealthyPeer(PeerId),
    /// A peer on a given connection that was previously unhealthy has returned
    /// to a healthy state.
    HealthyPeer(PeerId),
    /// A connection with a peer has dropped. The last state that was negotiated
    /// with the peer is also returned.
    PeerDisconnected(PeerId, NegotiatedPeerState),
    /// An outbound connection request was successfully negotiated with the
    /// remote peer.
    OutboundConnectionUpgradeSucceeded(PeerId),
    /// An outbound connection request failed to be upgraded, meaning the peer
    /// is a remote core but something failed when negotiating Blend protocol
    /// support.
    OutboundConnectionUpgradeFailed(PeerId),
}

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider> {
    #[must_use]
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Membership<PeerId>,
        local_peer_id: PeerId,
    ) -> Self {
        Self {
            sessions: Sessions::new(current_membership, *config.peering_degree.end()),
            events: VecDeque::new(),
            waker: None,
            observation_window_clock_provider,
            peering_degree: config.peering_degree.clone(),
            local_peer_id,
        }
    }

    /// Starts a new session with the provided membership.
    pub fn start_new_session(&mut self, new_membership: Membership<PeerId>) {
        if let Some(prev_session) = self
            .sessions
            .transition(new_membership, *self.peering_degree.end())
        {
            // If there was a previous session that was not closed yet, close it.
            // This can happen if a new session was started before the session
            // transition period has passed
            // (i.e. before `close_previous_session` is called).
            self.close_session(&prev_session);
        }
    }

    /// Closes the previous session by closing all its negotiated connections.
    /// This should be called once the session transition period has passed.
    pub fn close_previous_session(&mut self) {
        if let Some(session) = self.sessions.close_previous_session() {
            self.close_session(&session);
        }
    }

    /// Closes all negotiated connections in the session.
    fn close_session(&mut self, session: &Session) {
        session.negotiated_peers().for_each(
            |(&peer_id, RemotePeerConnectionDetails { connection_id, .. })| {
                self.events.push_back(ToSwarm::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::One(*connection_id),
                    event: Either::Left(FromBehaviour::CloseSubstreams),
                });
                self.try_wake();
            },
        );
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Publish an already-encapsulated message to all connected peers
    /// in the current session.
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
        self.publish_validated_message(&validated_message)
    }

    /// Publish an already-encapsulated message to all connected peers
    /// in the current session.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    pub fn publish_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
    ) -> Result<(), Error> {
        self.current_session_behaviour()
            .forward_validated_message_and_maybe_exclude(message, None)
    }

    /// Forwards a message to all healthy connections in the session
    /// that contains the specified `excluded_peer` in its negotiated peers.
    ///
    /// The message is not forwarded to the `excluded_peer`.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers that support
    /// the blend protocol.
    pub fn forward_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        (excluded_peer_id, excluded_conn_id): (PeerId, ConnectionId),
    ) -> Result<(), Error> {
        self.session_behaviour_for_conn(&(excluded_peer_id, excluded_conn_id))
            .ok_or(Error::NoSessionForPeer {
                peer_id: excluded_peer_id,
                connection_id: excluded_conn_id,
            })?
            .common()
            .forward_validated_message_and_maybe_exclude(message, Some(excluded_peer_id))
    }

    #[must_use]
    pub fn num_healthy_peers(&self) -> usize {
        self.sessions
            .current_session()
            .negotiated_peers()
            .filter(|(_, state)| state.negotiated_state == NegotiatedPeerState::Healthy)
            .count()
    }

    pub const fn minimum_healthy_peering_degree(&self) -> usize {
        *self.peering_degree.start()
    }

    #[must_use]
    pub fn available_connection_slots(&self) -> usize {
        self.sessions.current_session().available_connection_slots()
    }

    /// Handle a new negotiated connection.
    ///
    /// If the connection is present in the set of pending connections
    /// in the **current** session, it is processed by the following logic:
    /// - If this peer has already a connection with the connecting peer, the
    ///   connection selection logic will be run.
    /// - Otherwise, the new connection will be accepted as long as this peer
    ///   does not have the maximum number of connections already established.
    /// - Regardless of which road is taken, the connection is removed from the
    ///   set of pending connections since it has now been processed.
    ///
    /// If the connection is present in the set of pending connections
    /// in the **previous** session, it is removed from the set of pending
    /// connections, but its substream is closed.
    ///
    /// If the connection is not present in either of the sessions,
    /// it is ignored.
    fn handle_negotiated_connection(&mut self, conn: (PeerId, ConnectionId)) {
        match self.session_behaviour_for_pending_upgrade(&conn) {
            Some((SessionBehaviour::Current(mut behaviour), role)) => {
                behaviour.handle_negotiated_connection(conn, role);
            }
            Some((SessionBehaviour::Previous(mut behaviour), _)) => {
                behaviour.handle_negotiated_connection(conn);
            }
            None => {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Pending upgrade {conn:?} not found. Ignoring it.",
                );
            }
        }
    }

    /// Handle a spammy connection if it is part of the current session.
    /// If not, it is ignored.
    fn handle_spammy_connection(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        if let Some(mut behaviour) =
            self.current_session_behaviour_for_conn(&(peer_id, connection_id))
        {
            // We do not explicitly close the connection here since the connection handler
            // will already do that for us.
            behaviour.set_connection_to_spammy((peer_id, connection_id));
        }
    }

    /// Handle an unhealthy connection if it is part of the current session.
    /// If not, it is ignored.
    fn handle_unhealthy_connection(&mut self, connection: (PeerId, ConnectionId)) {
        if let Some(mut behaviour) = self.current_session_behaviour_for_conn(&connection) {
            behaviour.handle_unhealthy_connection(connection);
        }
    }

    /// Handle a connection that became healthy if it is part of the current session.
    /// If not, it is ignored.
    fn handle_healthy_connection(&mut self, connection: (PeerId, ConnectionId)) {
        if let Some(mut behaviour) = self.current_session_behaviour_for_conn(&connection) {
            behaviour.handle_healthy_connection(connection);
        }
    }

    /// Handle a message received from a peer,
    /// regardless of whether it is part of the current session or not.
    fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        from_connection: (PeerId, ConnectionId),
    ) {
        let Some(mut behaviour) = self.session_behaviour_for_conn(&from_connection) else {
            tracing::warn!(target: LOG_TARGET, "No session found for {from_connection:?}");
            return;
        };
        behaviour
            .common()
            .handle_received_serialized_encapsulated_message(serialized_message, from_connection);
    }

    /// Creates [`CurrentSessionBehaviour`].
    const fn current_session_behaviour(&mut self) -> CurrentSessionBehaviour<'_> {
        current_session_behaviour!(self, self.sessions.current_session_mut())
    }

    /// Creates [`SessionBehaviour`] for the session that has the given negotiated connection.
    fn session_behaviour_for_conn(
        &mut self,
        conn: &(PeerId, ConnectionId),
    ) -> Option<SessionBehaviour<'_>> {
        match self.sessions.session_mut_for_conn(conn)? {
            SessionRef::Current(session) => Some(SessionBehaviour::Current(
                current_session_behaviour!(self, session),
            )),
            SessionRef::Previous(session) => Some(SessionBehaviour::Previous(
                previous_session_behaviour!(self, session),
            )),
        }
    }

    /// Creates [`CurrentSessionBehaviour`] if the given connection is part of it.
    /// It returns [`None`] if the connection is not part of the current session.
    fn current_session_behaviour_for_conn(
        &mut self,
        conn: &(PeerId, ConnectionId),
    ) -> Option<CurrentSessionBehaviour<'_>> {
        let session = self.sessions.current_session_mut();
        session
            .is_negotiated_conn(conn)
            .then_some(current_session_behaviour!(self, session))
    }

    /// Creates [`SessionBehaviour`] for the session that has the given negotiated connection.
    fn session_behaviour_for_pending_upgrade(
        &mut self,
        conn: &(PeerId, ConnectionId),
    ) -> Option<(SessionBehaviour<'_>, Endpoint)> {
        let (role, session) = self.sessions.remove_connection_waiting_upgrade(conn)?;
        match session {
            SessionRef::Current(session) => Some((
                SessionBehaviour::Current(current_session_behaviour!(self, session)),
                role,
            )),
            SessionRef::Previous(session) => Some((
                SessionBehaviour::Previous(previous_session_behaviour!(self, session)),
                role,
            )),
        }
    }
}

/// A temporary structure that is created from [`Behaviour`]
/// in order to perform the behaviour only for the specified session.
///
/// It defines common methods that can be used for both the current and previous session.
//
// TODO: Move this to a separate module for readability.
struct CommonSessionBehaviour<'behaviour> {
    session: &'behaviour mut Session,
    events: &'behaviour mut VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    waker: &'behaviour mut Option<Waker>,
}

impl CommonSessionBehaviour<'_> {
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
        let mut at_least_one_receiver = false;
        let peers = self
            .session
            .negotiated_peers_except(excluded_peer)
            // Exclude from the list of candidate peers any peer that is not in a healthy state.
            .filter(|(_, peer_state)| peer_state.negotiated_state == NegotiatedPeerState::Healthy)
            .map(|(peer_id, peer_details)| (*peer_id, *peer_details))
            .collect::<Vec<_>>();
        for (peer_id, RemotePeerConnectionDetails { connection_id, .. }) in &peers {
            if self
                .session
                .add_exchanged_message_identifier(message_id, *peer_id)
            {
                tracing::debug!(target: LOG_TARGET, "Notifying handler with peer {peer_id:?} on connection {connection_id:?} to deliver message.");
                self.events.push_back(ToSwarm::NotifyHandler {
                    peer_id: *peer_id,
                    handler: NotifyHandler::One(*connection_id),
                    event: Either::Left(FromBehaviour::Message(serialized_message.clone())),
                });
                at_least_one_receiver = true;
            } else {
                tracing::trace!(target: LOG_TARGET, "Not sending message to peer {peer_id:?} because we already exchanged this message with them.");
            }
        }

        if at_least_one_receiver {
            self.try_wake();
            Ok(())
        } else {
            Err(Error::NoPeers)
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Notify the handler of the provided connection to close all its
    /// substreams. Leaving it up to the swarm to decide what to do with the
    /// connection.
    ///
    /// This function does not perform any checks to verify whether the
    /// specified connection is stored or not.
    fn close_substream(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        self.events.push_back(ToSwarm::NotifyHandler {
            peer_id,
            handler: NotifyHandler::One(connection_id),
            event: Either::Left(FromBehaviour::CloseSubstreams),
        });
        self.try_wake();
    }

    /// Mark the connection with the sender of a malformed message as malicious
    /// and instruct its connection handler to drop the substream.
    fn close_spammy_connection(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        tracing::debug!(target: LOG_TARGET, "Closing connection {connection_id:?} with spammy peer {peer_id:?}.");
        self.set_connection_to_spammy((peer_id, connection_id));
        self.close_substream((peer_id, connection_id));
    }

    /// Update the state of an already negotiated peer, returning the previous
    /// state.
    ///
    /// # Panics
    ///
    /// If there is no entry with the specified peer ID in the map of negotiated
    /// peers.
    fn update_state_for_negotiated_peer(
        &mut self,
        peer: (PeerId, ConnectionId),
        state: NegotiatedPeerState,
    ) -> NegotiatedPeerState {
        tracing::debug!(target: LOG_TARGET, "Marking peer {peer:?} as {state:?}.");
        self.session
            .update_negotiated_peer_state(&peer, state)
            .unwrap_or_else(|| panic!("Connection with {peer:?} not found in storage."))
    }

    fn set_connection_to_spammy(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        self.update_state_for_negotiated_peer(
            (peer_id, connection_id),
            NegotiatedPeerState::Spammy,
        );
    }

    fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        (from_peer_id, from_connection_id): (PeerId, ConnectionId),
    ) {
        // Mark a peer as malicious if it sends a un-deserializable message: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df8172927bebb75d8b988e.
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            tracing::debug!(target: LOG_TARGET, "Failed to deserialize encapsulated message.");
            self.close_spammy_connection((from_peer_id, from_connection_id));
            return;
        };

        let message_identifier = deserialized_encapsulated_message.id();

        // Mark a core peer as malicious if it sends a duplicate message maliciously (i.e., if a message with the same identifier was already exchanged with them): https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81fc86bdce264466efd3.
        let Ok(()) = self.check_and_update_message_cache(
            &message_identifier,
            (from_peer_id, from_connection_id),
        ) else {
            return;
        };
        // Verify the message public header, or else mark the peer as malicious: https://www.notion.so/nomos-tech/Blend-Protocol-Version-1-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81859cebf5e3d2a5cd8f.
        let Ok(validated_message) = deserialized_encapsulated_message.validate_public_header()
        else {
            tracing::debug!(target: LOG_TARGET, "Neighbor sent us a message with an invalid public header. Marking it as spammy.");
            self.close_spammy_connection((from_peer_id, from_connection_id));
            return;
        };

        // Notify the swarm about the received message, so that it can be further
        // processed by the core protocol module.
        self.events.push_back(ToSwarm::GenerateEvent(Event::Message(
            Box::new(validated_message),
            (from_peer_id, from_connection_id),
        )));
        self.try_wake();
    }

    fn check_and_update_message_cache(
        &mut self,
        message_id: &MessageIdentifier,
        (peer_id, connection_id): (PeerId, ConnectionId),
    ) -> Result<(), ()> {
        if !self
            .session
            .add_exchanged_message_identifier(*message_id, peer_id)
        {
            tracing::debug!(target: LOG_TARGET, "Neighbor {peer_id:?} on connection {connection_id:?} sent us a message previously already exchanged. Marking it as spammy.");
            self.close_spammy_connection((peer_id, connection_id));
            return Err(());
        }
        Ok(())
    }

    fn remove_negotiated_peer(&mut self, peer_id: &PeerId) -> Option<RemotePeerConnectionDetails> {
        self.session.remove_negotiated_peer(peer_id)
    }
}

/// Represent either the current or the previous session behaviour.
enum SessionBehaviour<'behaviour> {
    Current(CurrentSessionBehaviour<'behaviour>),
    Previous(PreviousSessionBehaviour<'behaviour>),
}

impl<'behaviour> SessionBehaviour<'behaviour> {
    /// Returns a mutable reference to the [`CommonSessionBehaviour`].
    fn common(&'behaviour mut self) -> &'behaviour mut CommonSessionBehaviour<'behaviour> {
        match self {
            Self::Current(behaviour) => behaviour,
            Self::Previous(behaviour) => behaviour,
        }
    }
}

/// A session behaviour for the previous session.
//
// TODO: Move this to a separate module for readability.
struct PreviousSessionBehaviour<'behaviour>(CommonSessionBehaviour<'behaviour>);

impl<'behaviour> Deref for PreviousSessionBehaviour<'behaviour> {
    type Target = CommonSessionBehaviour<'behaviour>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PreviousSessionBehaviour<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PreviousSessionBehaviour<'_> {
    /// Should be called when a negotiated connection is detected.
    fn handle_negotiated_connection(&mut self, conn: (PeerId, ConnectionId)) {
        self.close_substream(conn);
    }

    /// Should be called when a negotiated connection is closed.
    fn handle_closed_negotiated_connection(&mut self, peer_id: &PeerId) {
        self.remove_negotiated_peer(peer_id);
    }
}

/// A session behaviour for the current session.
//
// TODO: Move this to a separate module for readability.
struct CurrentSessionBehaviour<'behaviour> {
    common: CommonSessionBehaviour<'behaviour>,
    local_peer_id: PeerId,
}

impl<'behaviour> Deref for CurrentSessionBehaviour<'behaviour> {
    type Target = CommonSessionBehaviour<'behaviour>;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl DerefMut for CurrentSessionBehaviour<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

impl CurrentSessionBehaviour<'_> {
    /// Should be called when a new negotiated connection is detected.
    fn handle_negotiated_connection(
        &mut self,
        (peer_id, connection_id): (PeerId, ConnectionId),
        role: Endpoint,
    ) {
        if self.session.is_negotiated_peer(&peer_id) {
            self.handle_negotiated_connection_for_existing_peer((peer_id, connection_id), role);
        } else {
            self.handle_negotiated_connection_for_new_peer((peer_id, connection_id), role);
        }
    }

    /// Handle a newly upgraded connection for a peer that this peer is not
    /// already connected to.
    ///
    /// If this peer has already reached its maximum peering degree, the
    /// connection will be discarded.
    ///
    /// This function assumes that no entry for the provided peer ID is present
    /// in the map of already upgraded connections.
    fn handle_negotiated_connection_for_new_peer(
        &mut self,
        (peer_id, connection_id): (PeerId, ConnectionId),
        peer_role: Endpoint,
    ) {
        // We need to check if we still have available connection slots, as it is
        // possible, especially upon session transition, that more than the maximum
        // allowed number of peers are trying to connect to us. So once the stream is
        // actually upgraded, we downgrade it again if we do not have space left for it.
        // By not adding the new connection to the map of negotiated peers, the swarm
        // will not be notified about this dropped connection, which is what we want.
        if self.session.available_connection_slots() == 0 {
            tracing::debug!(target: LOG_TARGET, "Connection {connection_id:?} with peer {peer_id:?} must be closed because peering degree limit has already been reached.");
            self.close_substream((peer_id, connection_id));
            return;
        }
        debug_assert!(
            !self.session.is_negotiated_peer(&peer_id),
            "We are assuming the peer is not connected to us."
        );
        tracing::debug!(target: LOG_TARGET, "Connection {connection_id:?} with peer {peer_id:?} has been negotiated.");
        self.session.add_negotiated_peer(
            peer_id,
            RemotePeerConnectionDetails {
                role: peer_role,
                negotiated_state: NegotiatedPeerState::Healthy,
                connection_id,
            },
        );
        // If the new negotiated connection is outgoing, notify the Swarm about it.
        if peer_role == Endpoint::Listener {
            self.events.push_back(ToSwarm::GenerateEvent(
                Event::OutboundConnectionUpgradeSucceeded(peer_id),
            ));
            self.try_wake();
        }
    }

    /// Handle a newly upgraded connection for a peer that this peer is already
    /// connected to.
    ///
    /// Depending on the outcome of comparing the two peers' IDs, either the
    /// existing connection is replaced with the new one, or the new one is
    /// discarded in favor of the existing one.
    ///
    /// # Panics
    ///
    /// If there is no negotiated connection for the given peer in the relative
    /// storage.
    fn handle_negotiated_connection_for_existing_peer(
        &mut self,
        (peer_id, new_connection_id): (PeerId, ConnectionId),
        new_role: Endpoint,
    ) {
        let existing_connection = self.session.negotiated_peer(&peer_id)
            .unwrap_or_else(|| {
                panic!(
                    "Currently established connection with peer {peer_id:?} not found in storage of established connections.",
                )
            });
        match (existing_connection.role, new_role) {
            // Same connection direction (in case it was not caught at connection establishment
            // time), we ignore the new connection.
            (Endpoint::Dialer, Endpoint::Dialer) | (Endpoint::Listener, Endpoint::Listener) => {
                self.handle_connected_peer_duplicate_connection((peer_id, new_connection_id));
            }
            (Endpoint::Listener, Endpoint::Dialer) | (Endpoint::Dialer, Endpoint::Listener) => {
                self.handle_connected_peer_reverse_connection((peer_id, new_connection_id));
            }
        }
    }

    /// Close the new connection since there is already an established one in
    /// the same direction.
    fn handle_connected_peer_duplicate_connection(
        &mut self,
        (peer_id, new_connection_id): (PeerId, ConnectionId),
    ) {
        tracing::trace!(target: LOG_TARGET, "Connection {new_connection_id:?} with peer {peer_id:?} will be closed since there is already a connection established in the same direction.");
        self.close_substream((peer_id, new_connection_id));
    }

    /// Decide which connection to keep between an established one and
    /// a new incoming one.
    ///
    /// Depending on the outcome of comparing the two peers' IDs, either the
    /// existing connection is replaced with the new one, or the new one is
    /// discarded in favor of the existing one.
    fn handle_connected_peer_reverse_connection(
        &mut self,
        (peer_id, new_connection_id): (PeerId, ConnectionId),
    ) {
        let local_peer_id = self.local_peer_id;
        let existing_connection_details = self.session
            .negotiated_peer_mut(&peer_id)
            .unwrap_or_else(|| {
                panic!(
                    "Currently established connection with peer {peer_id:?} not found in storage of established connections.",
                )
            });
        // If the current connection is incoming, we close it if our peer ID is higher
        // than theirs.
        let should_close_established = if existing_connection_details.role == Endpoint::Dialer {
            local_peer_id.to_base58() > peer_id.to_base58()
        } else {
            // If the current connection is outgoing, we close it if our peer ID is lower
            // than theirs.
            local_peer_id.to_base58() <= peer_id.to_base58()
        };

        if should_close_established {
            tracing::trace!(target: LOG_TARGET, "Replacing established connection {:?} with peer {peer_id:?} with upgraded connection {new_connection_id:?}.", existing_connection_details.connection_id);
            let existing_connection = (peer_id, existing_connection_details.connection_id);
            // Modify the `negotiated_peers` storage directly so
            // that when the old connection is dropped, the swarm is
            // not notified.
            update_connection_id_and_direction(existing_connection_details, new_connection_id);
            // After the old connection details have been updated with the new
            // ones, notify the Swarm that the new connection has been upgraded,
            // if the new connection is an outgoing one.
            if existing_connection_details.role == Endpoint::Listener {
                self.events.push_back(ToSwarm::GenerateEvent(
                    Event::OutboundConnectionUpgradeSucceeded(peer_id),
                ));
            }
            // Notify the current connection handler to drop the substreams.
            self.close_substream(existing_connection);
        } else {
            tracing::trace!(target: LOG_TARGET, "Dropping upgraded connection {new_connection_id:?} with peer {peer_id:?} in favor of currently established connection {:?}", existing_connection_details.connection_id);
            // Notify the new connection handler to drop the substreams, and we do not
            // alter the storage.
            self.close_substream((peer_id, new_connection_id));
        }
    }

    fn handle_unhealthy_connection(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        // Notify swarm only on first transition into unhealthy state.
        if self.update_state_for_negotiated_peer(
            (peer_id, connection_id),
            NegotiatedPeerState::Unhealthy,
        ) != NegotiatedPeerState::Unhealthy
        {
            tracing::debug!(target: LOG_TARGET, "Peer {peer_id:?} has been marked as unhealthy.");
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::UnhealthyPeer(peer_id)));
            self.try_wake();
        }
    }

    fn handle_healthy_connection(&mut self, (peer_id, connection_id): (PeerId, ConnectionId)) {
        // Notify swarm only on first transition into healthy state.
        if self.update_state_for_negotiated_peer(
            (peer_id, connection_id),
            NegotiatedPeerState::Healthy,
        ) != NegotiatedPeerState::Healthy
        {
            tracing::debug!(target: LOG_TARGET, "Peer {peer_id:?} has been marked as healthy.");
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::HealthyPeer(peer_id)));
            self.try_wake();
        }
    }

    fn handle_closed_negotiated_connection(&mut self, peer_id: &PeerId) {
        let details = self
            .remove_negotiated_peer(peer_id)
            .expect("Peer must exist");
        self.events
            .push_back(ToSwarm::GenerateEvent(Event::PeerDisconnected(
                *peer_id,
                details.negotiated_state,
            )));
        self.try_wake();
    }
}

/// Revert the direction of a connection and updates its ID with the provided
/// one.
fn update_connection_id_and_direction(
    existing_connection: &mut RemotePeerConnectionDetails,
    new_connection_id: ConnectionId,
) {
    existing_connection.role = if existing_connection.role == Endpoint::Dialer {
        Endpoint::Listener
    } else {
        Endpoint::Dialer
    };
    existing_connection.connection_id = new_connection_id;
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
        let session = self.sessions.current_session_mut();

        // TODO: Don't accept inbound conns if we are not a core node.
        //       Refactor Membership to contain the local node.
        //       https://github.com/logos-co/nomos/issues/1533

        // If the new peer makes the set of established connections too large, do not
        // try to upgrade the connection.
        if session.available_connection_slots() == 0 {
            tracing::trace!(target: LOG_TARGET, "Inbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since we are already at maximum peering capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If there is already an established inbound connection with the given peer,
        // do not try to upgrade the new one as we already have an inbound connection.
        // Otherwise, we let the connection upgrade, and we will close one of the two
        // connections depending on the comparison result of local and remote peer IDs.
        if let Some(RemotePeerConnectionDetails {
            role: Endpoint::Dialer,
            ..
        }) = session.negotiated_peer(&peer_id)
        {
            tracing::trace!(target: LOG_TARGET, "Inbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since there is already an inbound connection established.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        Ok(if session.is_core_node(&peer_id) {
            tracing::debug!(target: LOG_TARGET, "Upgrading inbound connection {connection_id:?} with core peer {peer_id:?}.");
            session.add_pending_upgrade((peer_id, connection_id), Endpoint::Dialer);
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
        let session = self.sessions.current_session_mut();

        // TODO: Give up conns if we are not a core node anymore.
        //       Refactor Membership to contain the local node.
        //       https://github.com/logos-co/nomos/issues/1533

        // If the new peer makes the set of established connections too large, do not
        // try to upgrade the connection.
        if session.available_connection_slots() == 0 {
            tracing::trace!(target: LOG_TARGET, "Outbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since we are already at maximum peering capacity.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        // If there is already an established outbound connection with the given peer,
        // do not try to upgrade the new one as we already have an outbound connection.
        // Otherwise, we let the connection upgrade, and we will close one of the two
        // connections depending on the comparison result of local and remote peer IDs.
        if let Some(RemotePeerConnectionDetails {
            role: Endpoint::Listener,
            ..
        }) = session.negotiated_peer(&peer_id)
        {
            tracing::trace!(target: LOG_TARGET, "Outbound connection {connection_id:?} with peer {peer_id:?} will not be upgraded since there is already an outbound connection established.");
            return Ok(Either::Right(DummyConnectionHandler));
        }

        Ok(if session.is_core_node(&peer_id) {
            tracing::debug!(target: LOG_TARGET, "Upgrading outbound connection {connection_id:?} with core peer {peer_id:?}.");
            session.add_pending_upgrade((peer_id, connection_id), Endpoint::Listener);
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
            // We notify the swarm of any outbound connection that failed to be upgraded,
            // if it is in the current session.
            if let Some((SessionBehaviour::Current(_), role)) =
                self.session_behaviour_for_pending_upgrade(&(peer_id, connection_id))
            {
                if role == Endpoint::Listener {
                    self.events.push_back(ToSwarm::GenerateEvent(
                        Event::OutboundConnectionUpgradeFailed(peer_id),
                    ));
                    self.try_wake();
                }
            }

            match self.session_behaviour_for_conn(&(peer_id, connection_id)) {
                Some(SessionBehaviour::Current(mut behaviour)) => {
                    behaviour.handle_closed_negotiated_connection(&peer_id);
                }
                Some(SessionBehaviour::Previous(mut behaviour)) => {
                    behaviour.handle_closed_negotiated_connection(&peer_id);
                }
                None => {
                    // We are closing a different connection for the same peer, so a
                    // connection we have either replaced with a new one or ignored
                    // in favor of the old one.
                    tracing::trace!(target: LOG_TARGET, "Closing replaced or ignored connection {connection_id:?} with peer {peer_id:?}.");
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
