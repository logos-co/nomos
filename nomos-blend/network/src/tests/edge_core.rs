use std::task::{Context, Poll};

use futures::StreamExt as _;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, SwarmEvent, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use nomos_blend_scheduling::membership::Membership;
use rand::rngs::OsRng;

use crate::edge::{self, handler::ToBehaviour, Error};

pub async fn edge_sender_swarm() -> (Swarm<edge::Behaviour<OsRng>>, Multiaddr) {
    swarm(edge::Behaviour::new(None, OsRng)).await
}

pub async fn no_message_edge_sender_swarm() -> (Swarm<NoMessageSendBehaviour>, Multiaddr) {
    swarm(NoMessageSendBehaviour {
        inner: edge::Behaviour::new(None, OsRng),
    })
    .await
}

async fn swarm<Behaviour>(behaviour: Behaviour) -> (Swarm<Behaviour>, Multiaddr)
where
    Behaviour: NetworkBehaviour,
{
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| behaviour)
        .unwrap()
        .build();
    let _ = swarm
        .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
        .unwrap();
    let listening_addr = wait_for_listening_address(&mut swarm).await;
    (swarm, listening_addr)
}

async fn wait_for_listening_address<Behaviour>(swarm: &mut Swarm<Behaviour>) -> Multiaddr
where
    Behaviour: NetworkBehaviour,
{
    loop {
        if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
            break address;
        }
    }
}

pub struct NoMessageSendBehaviour {
    inner: edge::Behaviour<OsRng>,
}

impl NoMessageSendBehaviour {
    pub fn send_message(&mut self, message: Vec<u8>) -> Result<(), Error> {
        self.inner.send_message(message)
    }

    pub fn set_membership(&mut self, membership: Membership<PeerId>) {
        self.inner.set_membership(membership);
    }
}

impl NetworkBehaviour for NoMessageSendBehaviour {
    type ConnectionHandler = <edge::Behaviour<OsRng> as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = <edge::Behaviour<OsRng> as NetworkBehaviour>::ToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        if matches!(event, ToBehaviour::ReadyToSend) {
            // Do not send messages in this behaviour.
            return;
        }
        self.inner
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.inner.poll(cx)
    }
}
