use core::task::{Context, Poll, Waker};
use std::collections::VecDeque;

use futures::StreamExt as _;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, NotifyHandler, SwarmEvent,
        THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Swarm, SwarmBuilder,
};

use crate::handler::edge::{edge_core::FromBehaviour, EdgeToCoreBlendConnectionHandler};

pub(super) struct TestEdgeSenderBehaviour {
    events_from_handler: VecDeque<THandlerOutEvent<Self>>,
    events_to_handler: VecDeque<THandlerInEvent<Self>>,
    waker: Option<Waker>,
    peer_details: Option<(PeerId, ConnectionId)>,
}

impl TestEdgeSenderBehaviour {
    pub(super) fn new() -> Self {
        Self {
            events_from_handler: VecDeque::new(),
            events_to_handler: VecDeque::new(),
            waker: None,
            peer_details: None,
        }
    }

    pub(super) fn send_message(&mut self, message: Vec<u8>) {
        self.events_to_handler
            .push_back(FromBehaviour::Message(message));
    }
}

impl NetworkBehaviour for TestEdgeSenderBehaviour {
    type ConnectionHandler = EdgeToCoreBlendConnectionHandler;
    type ToSwarm = THandlerOutEvent<Self>;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Err(ConnectionDenied::new("Inbound connection denied"))
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.peer_details = Some((peer, connection_id));
        Ok(EdgeToCoreBlendConnectionHandler::new())
    }

    fn on_swarm_event(&mut self, _event: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.events_from_handler.push_back(event);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        let Some(peer_details) = self.peer_details else {
            return Poll::Pending;
        };
        if let Some(event_to_handler) = self.events_to_handler.pop_front() {
            return Poll::Ready(ToSwarm::NotifyHandler {
                peer_id: peer_details.0,
                handler: NotifyHandler::One(peer_details.1),
                event: event_to_handler,
            });
        }
        if let Some(event_from_handler) = self.events_from_handler.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(event_from_handler));
        }
        self.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub async fn edge_sender_swarm() -> (Swarm<TestEdgeSenderBehaviour>, Multiaddr) {
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| TestEdgeSenderBehaviour::new())
        .unwrap()
        .build();
    let _ = swarm
        .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
        .unwrap();
    let listening_addr = wait_for_listening_address(&mut swarm).await;
    (swarm, listening_addr)
}

async fn wait_for_listening_address(swarm: &mut Swarm<TestEdgeSenderBehaviour>) -> Multiaddr {
    loop {
        if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
            break address;
        }
    }
}
