use core::{
    task::{Context, Poll, Waker},
    time::Duration,
};
use std::collections::VecDeque;

use futures::StreamExt as _;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, SwarmEvent, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Swarm, SwarmBuilder,
};

use crate::handler::edge::CoreToEdgeBlendConnectionHandler;

pub(super) struct TestCoreReceiverBehaviour {
    timeout: Duration,
    events_from_handler: VecDeque<THandlerOutEvent<Self>>,
    waker: Option<Waker>,
    peer_details: Option<(PeerId, ConnectionId)>,
}

impl TestCoreReceiverBehaviour {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            events_from_handler: VecDeque::new(),
            timeout,
            waker: None,
            peer_details: None,
        }
    }
}

impl NetworkBehaviour for TestCoreReceiverBehaviour {
    type ConnectionHandler = CoreToEdgeBlendConnectionHandler;
    type ToSwarm = THandlerOutEvent<Self>;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.peer_details = Some((peer, connection_id));
        Ok(CoreToEdgeBlendConnectionHandler::new(self.timeout))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Err(ConnectionDenied::new("Outbound connection denied"))
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
        if let Some(event) = self.events_from_handler.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(event));
        }
        self.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub async fn core_receiver_swarm(
    timeout: Duration,
) -> (Swarm<TestCoreReceiverBehaviour>, Multiaddr) {
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| TestCoreReceiverBehaviour::new(timeout))
        .unwrap()
        .build();
    let _ = swarm
        .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
        .unwrap();
    let listening_addr = wait_for_listening_address(&mut swarm).await;
    (swarm, listening_addr)
}

async fn wait_for_listening_address(swarm: &mut Swarm<TestCoreReceiverBehaviour>) -> Multiaddr {
    loop {
        if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
            break address;
        }
    }
}
