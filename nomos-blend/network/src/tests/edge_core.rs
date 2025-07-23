use std::task::{Context, Poll};

use futures::StreamExt as _;
use libp2p::{
    core::{
        transport::PortUse,
        upgrade::{DeniedUpgrade, ReadyUpgrade},
        Endpoint,
    },
    swarm::{
        handler::{ConnectionEvent, FullyNegotiatedOutbound},
        ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId, FromSwarm,
        NetworkBehaviour, SubstreamProtocol, SwarmEvent, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol, Swarm, SwarmBuilder,
};
use nomos_blend_scheduling::membership::Membership;
use rand::rngs::OsRng;

use crate::{edge, PROTOCOL_NAME};

pub async fn edge_sender_swarm(
    membership: Option<Membership<PeerId>>,
) -> (Swarm<edge::Behaviour<OsRng>>, Multiaddr) {
    swarm(edge::Behaviour::new(membership, OsRng)).await
}

pub async fn dummy_edge_swarm() -> (Swarm<DummyBehaviour>, Multiaddr) {
    swarm(DummyBehaviour).await
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

/// A dummy [`NetworkBehaviour`] that creates a [`DummyConnectionHandler`]
/// for outbound connections.
pub struct DummyBehaviour;

impl NetworkBehaviour for DummyBehaviour {
    type ConnectionHandler = DummyConnectionHandler;
    type ToSwarm = ();

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Err(ConnectionDenied::new("Inbound not allowed"))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(DummyConnectionHandler::new())
    }

    fn on_swarm_event(&mut self, _event: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        _event: THandlerOutEvent<Self>,
    ) {
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

/// A dummy [`ConnectionHandler`] that creates a substream,
/// but doesn't sends any data through it.
pub struct DummyConnectionHandler {
    substream_requested: bool,
    substream: Option<Stream>,
}

impl DummyConnectionHandler {
    fn new() -> Self {
        Self {
            substream_requested: false,
            substream: None,
        }
    }
}

impl ConnectionHandler for DummyConnectionHandler {
    type FromBehaviour = ();
    type ToBehaviour = ();
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    #[expect(deprecated, reason = "Self::InboundOpenInfo is deprecated")]
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    #[expect(deprecated, reason = "Self::OutboundOpenInfo is deprecated")]
    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        // Opens a substream only once, on the first poll.
        if self.substream_requested {
            Poll::Pending
        } else {
            self.substream_requested = true;
            Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ()),
            })
        }
    }

    fn on_behaviour_event(&mut self, _event: Self::FromBehaviour) {}

    #[expect(
        deprecated,
        reason = "Self::InboundOpenInfo and Self::OutboundOpenInfo is deprecated"
    )]
    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        if let ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
            protocol: substream,
            ..
        }) = event
        {
            // Just keeps the substream open without sending any data.
            self.substream = Some(substream);
        }
    }
}
