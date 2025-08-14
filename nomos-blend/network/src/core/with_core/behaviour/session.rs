use std::{
    collections::{HashMap, HashSet},
    mem,
    ops::RangeInclusive,
};

use libp2p::{core::Endpoint, swarm::ConnectionId, PeerId};
use nomos_blend_message::MessageIdentifier;
use nomos_blend_scheduling::membership::Membership;

use crate::core::with_core::behaviour::{NegotiatedPeerState, RemotePeerConnectionDetails};

/// Represents the current and previous sessions.
pub struct Sessions {
    current: Session,
    previous: Option<Session>,
}

impl Sessions {
    pub fn new(membership: Membership<PeerId>) -> Self {
        Self {
            current: Session::new(membership),
            previous: None,
        }
    }

    /// Transitions to a new session with the given membership.
    ///
    /// It replaces the previous session with the current one,
    /// and sets the current session to the new one.
    ///
    /// If there was a previous session not closed yet, it will be returned.
    /// It can happen if the new session started before the session transition
    /// period has passed (i.e. before the previous session is closed).
    pub fn transition(&mut self, new_membership: Membership<PeerId>) -> Option<Session> {
        let previous = self.close_previous_session();
        self.previous = Some(mem::replace(
            &mut self.current,
            Session::new(new_membership),
        ));
        previous
    }

    /// Takes the previous session out of the [`Sessions`],
    /// leaving a [`None`] in its place.
    pub const fn close_previous_session(&mut self) -> Option<Session> {
        self.previous.take()
    }

    /// Returns a reference to the current session.
    pub const fn current_session(&self) -> &Session {
        &self.current
    }

    /// Returns a mutable reference to the current session.
    pub const fn current_session_mut(&mut self) -> &mut Session {
        &mut self.current
    }

    /// Returns a mutable reference to the session
    /// that has the given negotiated peer.
    pub fn session_mut(
        &mut self,
        negotiated_peer: &(PeerId, ConnectionId),
    ) -> Option<&mut Session> {
        if self.current.negotiated_conn(negotiated_peer).is_some() {
            return Some(&mut self.current);
        }

        if let Some(previous) = &mut self.previous {
            if previous.negotiated_conn(negotiated_peer).is_some() {
                return Some(previous);
            }
        }

        None
    }

    /// Removes the connection waiting for upgrade from either the current or
    /// previous session. It returns the [`Endpoint`] of the connection and
    /// the session it was found in.
    pub fn remove_connection_waiting_upgrade(
        &mut self,
        connection: &(PeerId, ConnectionId),
    ) -> Option<(Endpoint, &mut Session)> {
        if let Some(endpoint) = self.current.connections_waiting_upgrade.remove(connection) {
            return Some((endpoint, &mut self.current));
        }

        if let Some(previous) = &mut self.previous {
            if let Some(endpoint) = previous.connections_waiting_upgrade.remove(connection) {
                return Some((endpoint, previous));
            }
        }

        None
    }

    /// Checks if the given peer has been negotiated in either the current or
    /// previous session.
    pub fn is_negotiated(&self, peer_id: PeerId) -> bool {
        self.current.negotiated_peer(&peer_id).is_some()
            || self
                .previous
                .as_ref()
                .and_then(|s| s.negotiated_peer(&peer_id))
                .is_some()
    }
}

/// Represents a single session.
pub struct Session {
    membership: Membership<PeerId>,
    /// Tracks connections between this node and other core nodes.
    ///
    /// Only connections with other core nodes that are established before the
    /// specified connection limit is reached will be upgraded and the state of
    /// the peer negotiated, monitored, and reported to the swarm.
    negotiated_peers: HashMap<PeerId, RemotePeerConnectionDetails>,
    /// The set of connections established but not yet upgraded.
    ///
    /// We use this to keep track of the role of the remote peer, to be used
    /// when deciding which connection to close when a duplicate connection to
    /// the same peer is detected.
    connections_waiting_upgrade: HashMap<(PeerId, ConnectionId), Endpoint>,
    /// The session-bound storage keeping track, for each peer, what message
    /// identifiers have been exchanged between them.
    /// Sending a message with the same identifier more than once results in
    /// the peer being flagged as malicious, and the connection dropped.
    exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
}

impl Session {
    fn new(membership: Membership<PeerId>) -> Self {
        let membership_size = membership.size();
        Self {
            membership,
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            exchanged_message_identifiers: HashMap::with_capacity(membership_size),
        }
    }

    pub fn is_core_node(&self, peer_id: &PeerId) -> bool {
        self.membership.contains_remote(peer_id)
    }

    pub fn num_negotiated_peers(&self) -> usize {
        self.negotiated_peers.len()
    }

    pub fn available_connection_slots(&self, peering_degree: &RangeInclusive<usize>) -> usize {
        peering_degree
            .end()
            .saturating_sub(self.num_negotiated_peers())
    }

    pub fn negotiated_peers(
        &self,
    ) -> impl Iterator<Item = (&PeerId, &RemotePeerConnectionDetails)> + '_ {
        self.negotiated_peers_except(None)
    }

    pub fn negotiated_peers_except(
        &self,
        except: Option<PeerId>,
    ) -> impl Iterator<Item = (&PeerId, &RemotePeerConnectionDetails)> + '_ {
        self.negotiated_peers
            .iter()
            .filter(move |(&peer_id, _)| except != Some(peer_id))
    }

    pub fn negotiated_peer(&self, peer_id: &PeerId) -> Option<&RemotePeerConnectionDetails> {
        self.negotiated_peers.get(peer_id)
    }

    pub fn negotiated_peer_mut(
        &mut self,
        peer_id: &PeerId,
    ) -> Option<&mut RemotePeerConnectionDetails> {
        self.negotiated_peers.get_mut(peer_id)
    }

    fn negotiated_conn(
        &self,
        (peer_id, connection_id): &(PeerId, ConnectionId),
    ) -> Option<&RemotePeerConnectionDetails> {
        self.negotiated_peers
            .get(peer_id)
            .filter(|details| details.connection_id == *connection_id)
    }

    pub fn add_negotiated_peer(&mut self, peer_id: PeerId, details: RemotePeerConnectionDetails) {
        self.negotiated_peers.insert(peer_id, details);
    }

    pub fn update_negotiated_peer_state(
        &mut self,
        (peer_id, connection_id): &(PeerId, ConnectionId),
        state: NegotiatedPeerState,
    ) -> Option<NegotiatedPeerState> {
        self.negotiated_peers
            .get_mut(peer_id)
            .filter(|details| details.connection_id == *connection_id)
            .map(|details| mem::replace(&mut details.negotiated_state, state))
    }

    pub fn add_connection_waiting_upgrade(
        &mut self,
        conn: (PeerId, ConnectionId),
        endpoint: Endpoint,
    ) {
        self.connections_waiting_upgrade.insert(conn, endpoint);
    }

    /// Adds a message identifier as exchanged with a peer.
    /// Returns `false` if the identifier was previously added,
    pub fn add_exchanged_message_identifier(
        &mut self,
        message_identifier: MessageIdentifier,
        peer_id: PeerId,
    ) -> bool {
        self.exchanged_message_identifiers
            .entry(peer_id)
            .or_default()
            .insert(message_identifier)
    }

    /// Removes the negotiated peer from the session and returns its details.
    /// Also, removes the peer's exchanged message identifiers.
    pub fn remove_negotiated_peer(
        &mut self,
        peer_id: &PeerId,
    ) -> Option<RemotePeerConnectionDetails> {
        self.exchanged_message_identifiers.remove(peer_id);
        self.negotiated_peers.remove(peer_id)
    }
}
