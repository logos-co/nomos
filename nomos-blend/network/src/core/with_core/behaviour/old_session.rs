use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    convert::Infallible,
    mem,
    task::{Context, Poll, Waker},
};

use either::Either;
use libp2p::{
    swarm::{ConnectionId, NotifyHandler, ToSwarm},
    PeerId,
};
use nomos_blend_message::MessageIdentifier;
use nomos_blend_scheduling::{deserialize_encapsulated_message, serialize_encapsulated_message};

use crate::{
    core::with_core::{
        behaviour::{handler::FromBehaviour, Event},
        error::Error,
    },
    message::ValidateMessagePublicHeader as _,
    EncapsulatedMessageWithValidatedPublicHeader,
};

/// Defines behaviours for processing messages from the old session
/// until the session transition period has passed.
#[derive(Default)]
pub struct OldSession {
    negotiated_peers: HashMap<PeerId, ConnectionId>,
    exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    waker: Option<Waker>,
}

impl OldSession {
    #[must_use]
    pub fn new() -> Self {
        Self {
            negotiated_peers: HashMap::new(),
            exchanged_message_identifiers: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
        }
    }

    /// Starts an old session with the given states.
    ///
    /// If it still has the old session that is not stopped yet, it stops it first.
    /// This can happen if this method was called before [`Self::stop`] is called.
    pub fn start(
        &mut self,
        negotiated_peers: HashMap<PeerId, ConnectionId>,
        exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    ) {
        self.stop();
        self.negotiated_peers = negotiated_peers;
        self.exchanged_message_identifiers = exchanged_message_identifiers;
    }

    /// Stops the old session, notifying all peers that it is closing substreams.
    ///
    /// It should be called once the session transition period has passed.
    pub fn stop(&mut self) {
        let peers = mem::take(&mut self.negotiated_peers);
        for (&peer_id, &connection_id) in &peers {
            self.events.push_back(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(connection_id),
                event: Either::Left(FromBehaviour::CloseSubstreams),
            });
            self.try_wake();
        }
        self.exchanged_message_identifiers.clear();
    }

    /// Checks if the connection is part of the old session.
    #[must_use]
    pub fn is_negotiated(&self, (peer_id, connection_id): &(PeerId, ConnectionId)) -> bool {
        self.negotiated_peers
            .get(peer_id)
            .is_some_and(|&id| id == *connection_id)
    }

    /// Forwards a message to all connections except the [`from`] peer.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers.
    pub fn forward_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        from: PeerId,
    ) -> Result<(), Error> {
        let message_id = message.id();
        let serialized_message = serialize_encapsulated_message(message);
        let mut at_least_one_receiver = false;
        self.negotiated_peers
            .iter()
            .filter(|(&peer_id, _)| peer_id != from)
            .for_each(|(&peer_id, &connection_id)| {
                if Self::check_and_update_message_cache(
                    &mut self.exchanged_message_identifiers,
                    &message_id,
                    peer_id,
                )
                .is_ok()
                {
                    self.events.push_back(ToSwarm::NotifyHandler {
                        peer_id,
                        handler: NotifyHandler::One(connection_id),
                        event: Either::Left(FromBehaviour::Message(serialized_message.clone())),
                    });
                    at_least_one_receiver = true;
                }
            });

        if at_least_one_receiver {
            self.try_wake();
            Ok(())
        } else {
            Err(Error::NoPeers)
        }
    }

    /// Handles a message received from a peer.
    pub fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        (from_peer_id, from_connection_id): (PeerId, ConnectionId),
    ) {
        // Deserialize the message.
        let Ok(deserialized_encapsulated_message) =
            deserialize_encapsulated_message(serialized_message)
        else {
            return;
        };

        let message_identifier = deserialized_encapsulated_message.id();

        // Add the message to the set of exchanged message identifiers.
        let Ok(()) = Self::check_and_update_message_cache(
            &mut self.exchanged_message_identifiers,
            &message_identifier,
            from_peer_id,
        ) else {
            return;
        };

        // Verify the message public header
        let Ok(validated_message) = deserialized_encapsulated_message.validate_public_header()
        else {
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
        exchanged_message_identifiers: &mut HashMap<PeerId, HashSet<MessageIdentifier>>,
        message_id: &MessageIdentifier,
        peer_id: PeerId,
    ) -> Result<(), ()> {
        if exchanged_message_identifiers
            .entry(peer_id)
            .or_default()
            .insert(*message_id)
        {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Should be called when a connection is detected as closed.
    ///
    /// It removes the connection from the states.
    pub fn handle_closed_connection(&mut self, (peer_id, connection_id): &(PeerId, ConnectionId)) {
        if let Entry::Occupied(entry) = self.negotiated_peers.entry(*peer_id) {
            if entry.get() == connection_id {
                entry.remove();
                self.exchanged_message_identifiers.remove(peer_id);
            }
        }
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    pub fn poll(
        &mut self,
        cx: &Context<'_>,
    ) -> Poll<ToSwarm<Event, Either<FromBehaviour, Infallible>>> {
        if let Some(event) = self.events.pop_front() {
            Poll::Ready(event)
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
