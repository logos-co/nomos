use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    convert::Infallible,
    task::{Context, Poll, Waker},
};

use either::Either;
use libp2p::{
    swarm::{ConnectionId, NotifyHandler, ToSwarm},
    PeerId,
};
use nomos_blend_message::{
    encap::{self, encapsulated::PoQVerificationInputMinusSigningKey},
    MessageIdentifier,
};
use nomos_blend_scheduling::{
    deserialize_encapsulated_message,
    message_blend::crypto::EncapsulatedMessageWithValidatedPublicHeader,
    serialize_encapsulated_message,
};

use crate::core::with_core::{
    behaviour::{handler::FromBehaviour, Event},
    error::Error,
};

/// Defines behaviours for processing messages from the old session
/// until the session transition period has passed.
pub struct OldSession<ProofsVerifier> {
    negotiated_peers: HashMap<PeerId, ConnectionId>,
    exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    waker: Option<Waker>,
    poq_verification_inputs: PoQVerificationInputMinusSigningKey,
    poq_verifier: ProofsVerifier,
}

impl<ProofsVerifier> OldSession<ProofsVerifier> {
    #[must_use]
    pub const fn new(
        negotiated_peers: HashMap<PeerId, ConnectionId>,
        exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
        poq_verification_inputs: PoQVerificationInputMinusSigningKey,
        poq_verifier: ProofsVerifier,
    ) -> Self {
        Self {
            negotiated_peers,
            exchanged_message_identifiers,
            events: VecDeque::new(),
            waker: None,
            poq_verification_inputs,
            poq_verifier,
        }
    }

    /// Stops the old session by returning events to close all the substreams
    /// in the old session.
    ///
    /// It should be called once the session transition period has passed.
    pub fn stop(self) -> VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>> {
        let mut events = VecDeque::with_capacity(self.negotiated_peers.len());
        for (&peer_id, &connection_id) in &self.negotiated_peers {
            events.push_back(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(connection_id),
                event: Either::Left(FromBehaviour::CloseSubstreams),
            });
        }
        events
    }

    /// Checks if the connection is part of the old session.
    #[must_use]
    fn is_negotiated(&self, (peer_id, connection_id): &(PeerId, ConnectionId)) -> bool {
        self.negotiated_peers
            .get(peer_id)
            .is_some_and(|&id| id == *connection_id)
    }

    /// Forwards a message to all connections except the [`except`] connection.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    ///
    /// # Returns
    /// - [`Ok(false)`] if the [`except`] connection is not part of the session.
    /// - [`Ok(true)`] if the message was forwarded to at least one peer.
    /// - [`Err(Error::NoPeers)`] if there are no other connected peers except
    ///   the [`except`] connection.
    pub fn forward_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        except: &(PeerId, ConnectionId),
    ) -> Result<bool, Error> {
        if !self.is_negotiated(except) {
            return Ok(false);
        }

        let message_id = message.id();
        let serialized_message = serialize_encapsulated_message(message);
        let mut at_least_one_receiver = false;
        self.negotiated_peers
            .iter()
            .filter(|(&peer_id, _)| peer_id != except.0)
            .for_each(|(&peer_id, &connection_id)| {
                if check_and_update_message_cache(
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
            Ok(true)
        } else {
            Err(Error::NoPeers)
        }
    }

    /// Should be called when a connection is detected as closed.
    ///
    /// It removes the connection from the states and returns [`true`]
    /// if the connection was part of the old session.
    pub fn handle_closed_connection(
        &mut self,
        (peer_id, connection_id): &(PeerId, ConnectionId),
    ) -> bool {
        if let Entry::Occupied(entry) = self.negotiated_peers.entry(*peer_id) {
            if entry.get() == connection_id {
                entry.remove();
                self.exchanged_message_identifiers.remove(peer_id);
                return true;
            }
        }
        false
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

fn check_and_update_message_cache(
    exchanged_message_identifiers: &mut HashMap<PeerId, HashSet<MessageIdentifier>>,
    message_id: &MessageIdentifier,
    peer_id: PeerId,
) -> Result<(), Error> {
    if exchanged_message_identifiers
        .entry(peer_id)
        .or_default()
        .insert(*message_id)
    {
        Ok(())
    } else {
        Err(Error::MessageAlreadyExchanged)
    }
}

impl<ProofsVerifier> OldSession<ProofsVerifier>
where
    ProofsVerifier: encap::ProofsVerifier,
{
    /// Handles a message received from a peer.
    ///
    /// # Returns
    /// - [`Ok(false)`] if the connection is not part of the session.
    /// - [`Ok(true)`] if the message was successfully processed and forwarded.
    /// - [`Err(Error)`] if the message is invalid or has already been
    ///   exchanged.
    pub fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        (from_peer_id, from_connection_id): (PeerId, ConnectionId),
    ) -> Result<bool, Error> {
        if !self.is_negotiated(&(from_peer_id, from_connection_id)) {
            return Ok(false);
        }

        // Deserialize the message.
        let deserialized_encapsulated_message =
            deserialize_encapsulated_message(serialized_message)
                .map_err(|_| Error::InvalidMessage)?;

        let message_identifier = deserialized_encapsulated_message.id();

        // Add the message to the set of exchanged message identifiers.
        check_and_update_message_cache(
            &mut self.exchanged_message_identifiers,
            &message_identifier,
            from_peer_id,
        )?;

        // Verify the message public header
        let validated_message = deserialized_encapsulated_message
            .verify_public_header(&self.poq_verification_inputs, &self.poq_verifier)
            .map_err(|_| Error::InvalidMessage)?;

        // Notify the swarm about the received message, so that it can be further
        // processed by the core protocol module.
        self.events.push_back(ToSwarm::GenerateEvent(Event::Message(
            Box::new(validated_message),
            (from_peer_id, from_connection_id),
        )));
        self.try_wake();
        Ok(true)
    }
}
