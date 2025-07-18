use std::{
    collections::VecDeque,
    task::{Context, Poll, Waker},
};

use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionDenied, ConnectionId, DialFailure, FromSwarm, NetworkBehaviour, NotifyHandler,
        THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nomos_blend_scheduling::membership::Membership;
use rand::RngCore;

use super::{
    error::Error,
    handler::{EdgeToCoreBlendConnectionHandler, FailureReason, FromBehaviour, ToBehaviour},
};

const LOG_TARGET: &str = "blend::network::edge::behaviour";

/// A [`NetworkBehaviour`] for an edge node,
/// which sends data messages to one of core nodes in the Blend network.
pub struct Behaviour<Rng> {
    /// Queue of events to yield to the swarm.
    events: VecDeque<ToSwarm<EventToSwarm, FromBehaviour>>,
    /// Pending messages to be sent once a new connection is established.
    pending_messages: VecDeque<Vec<u8>>,
    /// Waker that handles polling
    waker: Option<Waker>,
    // TODO: Replace with the session stream and make this a non-Option
    //       https://github.com/logos-co/nomos/issues/1462
    current_membership: Option<Membership<PeerId>>,
    rng: Rng,
}

#[derive(Debug)]
pub enum EventToSwarm {
    /// Notify the swarm that the message was sent successfully.
    MessageSuccess(Vec<u8>),
    /// Notify the swarm that the message could not be sent.
    SendError {
        reason: FailureReason,
        peer_id: PeerId,
        connection_id: ConnectionId,
    },
}

impl<Rng> Behaviour<Rng>
where
    Rng: RngCore,
{
    #[must_use]
    pub const fn new(current_membership: Option<Membership<PeerId>>, rng: Rng) -> Self {
        Self {
            events: VecDeque::new(),
            pending_messages: VecDeque::new(),
            waker: None,
            current_membership,
            rng,
        }
    }

    /// Schedules sending a message to a core node randomly selected.
    pub fn send_message(&mut self, message: Vec<u8>) -> Result<(), Error> {
        self.schedule_dial_random_node()?;

        // Add the message to the pending messages queue.
        // This will be sent once a connection is established.
        self.pending_messages.push_back(message);
        Ok(())
    }

    /// Schedules dialing to a random node.
    fn schedule_dial_random_node(&mut self) -> Result<(), Error> {
        let Some(membership) = &self.current_membership else {
            return Err(Error::NoPeers);
        };

        let node = membership
            .choose_remote_nodes(&mut self.rng, 1)
            .next()
            .ok_or(Error::NoPeers)?;

        self.events.push_back(ToSwarm::Dial {
            opts: DialOpts::peer_id(node.id)
                .condition(PeerCondition::DisconnectedAndNotDialing)
                .addresses(vec![node.address.clone()])
                .build(),
        });
        self.try_wake();
        Ok(())
    }

    /// Schedules sending a message to the peer that is connected.
    fn schedule_send_message(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        if let Some(message) = self.pending_messages.pop_front() {
            println!("SCHEDULE SEND MESSAGE");
            self.events.push_back(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(connection_id),
                event: FromBehaviour::Message(message),
            });
            self.try_wake();
        }
    }

    /// Schedules a [`EventToSwarm`] to be sent to the swarm.
    fn schedule_event_to_swarm(&mut self, event: EventToSwarm) {
        self.events.push_back(ToSwarm::GenerateEvent(event));
        self.try_wake();
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    // TODO: Remove this method once the session stream is implemented.
    //       https://github.com/logos-co/nomos/issues/1462
    pub fn set_membership(&mut self, membership: Membership<PeerId>) {
        self.current_membership = Some(membership);
    }
}

impl<Rng> NetworkBehaviour for Behaviour<Rng>
where
    Rng: RngCore + 'static,
{
    type ConnectionHandler = EdgeToCoreBlendConnectionHandler;
    type ToSwarm = EventToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Err(ConnectionDenied::new(
            ConnectionDeniedReason::InboundNotAllowed,
        ))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer_id: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // If no membership is provided (for tests), we assume all peers are core nodes.
        // nodes.
        let Some(membership) = &self.current_membership else {
            return Ok(EdgeToCoreBlendConnectionHandler::new());
        };
        if membership.contains_remote(&peer_id) {
            Ok(EdgeToCoreBlendConnectionHandler::new())
        } else {
            Err(ConnectionDenied::new(
                ConnectionDeniedReason::OutboundToEdgeNodeNotAllowed,
            ))
        }
    }

    /// Informs the behaviour about an event from the [`Swarm`].
    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::DialFailure(DialFailure { peer_id, error, .. }) = event {
            tracing::debug!(
                    "Failed to dial to peer:{peer_id:?}, error:{error:?}. Requesting another dialing..."
                );
            if let Err(e) = self.schedule_dial_random_node() {
                tracing::error!(target: LOG_TARGET, "Failed to request dialing to a random node: {e:?}");
            }
        }
    }

    /// Handles an event generated by the [`EdgeToCoreBlendConnectionHandler`]
    /// dedicated to the connection identified by `peer_id` and `connection_id`.
    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {
            ToBehaviour::ReadyToSend => {
                self.schedule_send_message(peer_id, connection_id);
            }
            ToBehaviour::MessageSuccess(message) => {
                self.schedule_event_to_swarm(EventToSwarm::MessageSuccess(message));
            }
            ToBehaviour::SendError(reason) => {
                self.schedule_event_to_swarm(EventToSwarm::SendError {
                    reason,
                    peer_id,
                    connection_id,
                });
            }
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

#[derive(Debug, thiserror::Error)]
enum ConnectionDeniedReason {
    #[error("Inbound connection not allowed for edge node")]
    InboundNotAllowed,
    #[error("Outbound connection to edge node is not allowed")]
    OutboundToEdgeNodeNotAllowed,
}
