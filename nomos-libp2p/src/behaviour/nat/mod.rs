use std::task::{Context, Poll};

use either::Either;
use libp2p::{
    autonat,
    core::{transport::PortUse, Endpoint},
    swarm::{
        behaviour::toggle::{Toggle, ToggleConnectionHandler},
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, NewListenAddr, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use rand::RngCore;

mod address_mapper;
mod gateway_monitor;
mod inner;
mod state_machine;

use crate::{
    behaviour::nat::inner::{InnerNatBehaviour, NatBehaviour},
    config::NatSettings,
};

/// This behaviour is responsible for confirming that the addresses of the node
/// are publicly reachable.
pub struct Behaviour<Rng: RngCore + 'static> {
    /// The static public listen address is passed through this variable to
    /// the `poll()` method. Unused if the node is not configured with a static
    /// public IP address.
    static_listen_addr: Option<Multiaddr>,
    /// Provides dynamic NAT-status detection, NAT-status improvement (via
    /// address mapping on the NAT-box), and periodic maintenance capabilities.
    /// Disabled if the node is configured with a static public IP address.
    inner_behaviour: Toggle<NatBehaviour<Rng>>,
}

impl<Rng: RngCore + 'static> Behaviour<Rng> {
    pub fn new(rng: Rng, nat_config: Option<NatSettings>) -> Self {
        let inner_behaviour =
            Toggle::from(nat_config.map(|config| InnerNatBehaviour::new(rng, config)));

        Self {
            static_listen_addr: None,
            inner_behaviour,
        }
    }
}

impl<Rng: RngCore + 'static> NetworkBehaviour for Behaviour<Rng> {
    type ConnectionHandler = ToggleConnectionHandler<
        <autonat::v2::client::Behaviour as NetworkBehaviour>::ConnectionHandler,
    >;

    type ToSwarm = Either<
        <autonat::v2::client::Behaviour as NetworkBehaviour>::ToSwarm,
        address_mapper::Event,
    >;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.inner_behaviour.handle_pending_inbound_connection(
            connection_id,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner_behaviour.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.inner_behaviour.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
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
        self.inner_behaviour.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let Some(inner_behaviour) = self.inner_behaviour.as_mut() {
            inner_behaviour.on_swarm_event(event);
        } else if let FromSwarm::NewListenAddr(NewListenAddr { addr, .. }) = event {
            self.static_listen_addr = Some(addr.clone());
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.inner_behaviour
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(addr) = self.static_listen_addr.take() {
            return Poll::Ready(ToSwarm::ExternalAddrConfirmed(addr));
        }

        if let Poll::Ready(to_swarm) = self.inner_behaviour.poll(cx) {
            return Poll::Ready(to_swarm);
        }

        Poll::Pending
    }
}
