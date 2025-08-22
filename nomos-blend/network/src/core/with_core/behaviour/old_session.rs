use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    convert::Infallible,
    marker::PhantomData,
    ops::RangeInclusive,
    task::{Context, Poll, Waker},
};

use either::Either;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler as DummyConnectionHandler, ConnectionClosed, ConnectionDenied,
        ConnectionId, FromSwarm, NotifyHandler, THandler, THandlerInEvent, THandlerOutEvent,
        ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_message::MessageIdentifier;
use nomos_blend_scheduling::{deserialize_encapsulated_message, serialize_encapsulated_message};
use nomos_libp2p::NetworkBehaviour;

use crate::{
    core::with_core::{
        behaviour::{
            handler::{ConnectionHandler, FromBehaviour, ToBehaviour},
            IntervalStreamProvider,
        },
        error::Error,
    },
    message::ValidateMessagePublicHeader as _,
    EncapsulatedMessageWithValidatedPublicHeader,
};

#[derive(Default)]
pub struct Behaviour<ObservationWindowClockProvider> {
    negotiated_peers: HashMap<PeerId, ConnectionId>,
    exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    events: VecDeque<ToSwarm<Event, Either<FromBehaviour, Infallible>>>,
    waker: Option<Waker>,
    _phantom: PhantomData<ObservationWindowClockProvider>,
}

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the core peers, after its public header
    /// has been verified.
    Message(Box<EncapsulatedMessageWithValidatedPublicHeader>, PeerId),
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
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // Don't accept any new connection.
        Ok(Either::Right(DummyConnectionHandler))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // Don't accept any new connection.
        Ok(Either::Right(DummyConnectionHandler))
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed {
            peer_id,
            connection_id,
            ..
        }) = event
        {
            if let Entry::Occupied(entry) = self.negotiated_peers.entry(peer_id) {
                if entry.get() == &connection_id {
                    entry.remove();
                    self.exchanged_message_identifiers.remove(&peer_id);
                }
            }
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        println!("OldSession event from connhandler: {event:?}");
        if let Either::Left(ToBehaviour::Message(message)) = event {
            self.handle_received_serialized_encapsulated_message(&message, peer_id);
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

impl<ObservationWindowClockProvider> Behaviour<ObservationWindowClockProvider> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            negotiated_peers: HashMap::new(),
            exchanged_message_identifiers: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            _phantom: PhantomData,
        }
    }

    pub fn start(
        &mut self,
        negotiated_peers: HashMap<PeerId, ConnectionId>,
        exchanged_message_identifiers: HashMap<PeerId, HashSet<MessageIdentifier>>,
    ) {
        self.stop();
        self.negotiated_peers = negotiated_peers;
        self.exchanged_message_identifiers = exchanged_message_identifiers;
    }

    pub fn stop(&mut self) {
        let mut event_added = false;
        self.negotiated_peers
            .iter()
            .for_each(|(&peer_id, &connection_id)| {
                self.events.push_back(ToSwarm::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::One(connection_id),
                    event: Either::Left(FromBehaviour::CloseSubstreams),
                });
                event_added = true;
            });
        if event_added {
            self.try_wake();
        }
        self.negotiated_peers.clear();
        self.exchanged_message_identifiers.clear();
    }

    /// Forwards a message to all connections except the specified one.
    ///
    /// Public header validation checks are skipped, since the message is
    /// assumed to have been properly formed.
    ///
    /// Returns [`Error::NoPeers`] if there are no connected peers.
    pub fn forward_validated_message(
        &mut self,
        message: &EncapsulatedMessageWithValidatedPublicHeader,
        exclude_peer: PeerId,
    ) -> Result<(), Error> {
        let message_id = message.id();
        let serialized_message = serialize_encapsulated_message(message);
        let mut at_least_one_receiver = false;
        self.negotiated_peers
            .iter()
            .filter(|(&peer_id, _)| peer_id != exclude_peer)
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

    fn handle_received_serialized_encapsulated_message(
        &mut self,
        serialized_message: &[u8],
        from_peer_id: PeerId,
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
            from_peer_id,
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

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}
