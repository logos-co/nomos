use std::{
    collections::VecDeque,
    convert::Infallible,
    task::{Context, Poll, Waker},
    time::Duration,
};

use either::Either;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler as DummyConnectionHandler, ConnectionDenied, ConnectionId,
        FromSwarm, NetworkBehaviour, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_scheduling::membership::Membership;

use crate::core::to_edge::behaviour::handler::{ConnectionHandler, ToBehaviour};

mod handler;

#[derive(Debug)]
pub enum Event {
    /// A message received from one of the peers.
    Message(Vec<u8>),
}

#[derive(Debug)]
pub struct Config {
    pub connection_timeout: Duration,
}

pub struct Behaviour {
    events: VecDeque<ToSwarm<Event, Either<Infallible, Infallible>>>,
    waker: Option<Waker>,
    current_membership: Option<Membership<PeerId>>,
    connection_timeout: Duration,
}

impl Behaviour {
    pub const fn new(config: &Config, current_membership: Option<Membership<PeerId>>) -> Self {
        Self {
            events: VecDeque::new(),
            waker: None,
            current_membership,
            connection_timeout: config.connection_timeout,
        }
    }

    fn create_connection_handler_for_remote_core(&self) -> ConnectionHandler {
        ConnectionHandler::new(self.connection_timeout)
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Either<ConnectionHandler, DummyConnectionHandler>;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        let Some(membership) = &self.current_membership else {
            return Ok(Either::Right(DummyConnectionHandler));
        };
        Ok(if membership.contains_remote(&peer) {
            Either::Left(self.create_connection_handler_for_remote_core())
        } else {
            Either::Right(DummyConnectionHandler)
        })
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // No outbound sub-stream at all.
        Ok(Either::Right(DummyConnectionHandler))
    }

    fn on_swarm_event(&mut self, _: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        if let Either::Left(ToBehaviour::Message(message)) = event {
            self.events
                .push_back(ToSwarm::GenerateEvent(Event::Message(message)));
        }
        self.try_wake();
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
