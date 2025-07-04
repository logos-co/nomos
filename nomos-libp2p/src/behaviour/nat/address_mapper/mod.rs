mod errors;
mod protocol;
mod upnp;

use std::task::{Context, Poll, Waker};

use futures::future::{BoxFuture, FutureExt as _};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dummy::ConnectionHandler, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour,
        THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use tracing::{info, warn};

use crate::behaviour::nat::address_mapper::{
    errors::AddressMapperError, protocol::ProtocolManager,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    AddressMappingFailed(Multiaddr),
    DefaultGatewayChanged,
    LocalAddressChanged(Multiaddr),
    NewExternalMappedAddress(Multiaddr),
}

/// UPnP-based TCP address mapper
#[derive(Default)]
pub struct AddressMapperBehaviour {
    mapping_future: Option<BoxFuture<'static, Result<Multiaddr, AddressMapperError>>>,
    original_address: Option<Multiaddr>,
    waker: Option<Waker>,
}

impl AddressMapperBehaviour {
    pub fn try_map_address(&mut self, address: Multiaddr) {
        if self.mapping_future.is_none() {
            self.start_mapping(address);

            if let Some(waker) = &self.waker {
                waker.wake_by_ref();
            }
        }
    }

    fn start_mapping(&mut self, address: Multiaddr) {
        self.original_address = Some(address.clone());

        let mapping_future = async move {
            let mut protocol_manager = ProtocolManager::initialize().await?;
            protocol_manager.try_map_address(&address).await
        }
        .boxed();

        self.mapping_future = Some(mapping_future);
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
        self.waker = Some(cx.waker().clone());

        let Some(mut mapping_future) = self.mapping_future.take() else {
            return Poll::Pending;
        };

        match mapping_future.as_mut().poll(cx) {
            Poll::Ready(Ok(external_addr)) => {
                info!("Successfully mapped to external address: {}", external_addr);

                Poll::Ready(ToSwarm::GenerateEvent(Event::NewExternalMappedAddress(
                    external_addr,
                )))
            }
            Poll::Ready(Err(error)) => {
                warn!("Failed to map address: {}", error);

                let failed_addr = self
                    .original_address
                    .take()
                    .expect("Original address must be set");

                Poll::Ready(ToSwarm::GenerateEvent(Event::AddressMappingFailed(
                    failed_addr,
                )))
            }
            Poll::Pending => {
                self.mapping_future = Some(mapping_future);
                Poll::Pending
            }
        }
    }
}
