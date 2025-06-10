use std::task::{Context, Poll, Waker};

use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour,
        THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};

/// This is a placeholder for a behaviour that would map the node's addresses at
/// the default gateway using one of the protocols: `PCP`, `NAT-PMP`,
/// `UPNP-IGD`.
///
/// The current implementation does not perform any actual address mapping, and
/// always generates a failure event.
pub struct AddressMapperBehaviour {
    address_to_map: Option<Multiaddr>,
    waker: Option<Waker>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    AddressMappingFailed(Multiaddr),
    _DefaultGatewayChanged,
    _LocalAddressChanged(Multiaddr),
    _NewExternalMappedAddress(Multiaddr),
}

impl AddressMapperBehaviour {
    pub fn new() -> Self {
        AddressMapperBehaviour {
            address_to_map: None,
            waker: None,
        }
    }

    /// Instruct the behaviour to attempt to map the given address.
    pub fn try_map_address(&mut self, address: Multiaddr) {
        self.address_to_map = Some(address);
        self.try_wake();
    }

    pub fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl NetworkBehaviour for AddressMapperBehaviour {
    type ConnectionHandler = ConnectionHandler;

    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        _: THandlerOutEvent<Self>,
    ) {
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(address) = self.address_to_map.take() {
            // For now we don't perform any actual mapping, so just generate a failure
            // event.
            return Poll::Ready(ToSwarm::GenerateEvent(Event::AddressMappingFailed(address)));
        }

        self.waker = Some(cx.waker().clone());

        Poll::Pending
    }
}
