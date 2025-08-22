use libp2p::{
    autonat,
    swarm::{behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate},
    Multiaddr,
};

use crate::behaviour::nat::address_mapper;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    AutonatClientTestFailed(Multiaddr),
    AutonatClientTestOk(Multiaddr),
    AddressMappingFailed(Multiaddr),
    DefaultGatewayChanged(Option<Multiaddr> /* gateway lost / new gateway addr */),
    ExternalAddressConfirmed(Multiaddr),
    LocalAddressChanged(Multiaddr),
    NewExternalAddressCandidate(Multiaddr),
    NewExternalMappedAddress {
        local_address: Multiaddr,
        external_address: Multiaddr,
    },
}

impl TryFrom<&autonat::v2::client::Event> for Event {
    type Error = ();

    fn try_from(event: &autonat::v2::client::Event) -> Result<Self, Self::Error> {
        Ok(match event {
            autonat::v2::client::Event {
                result: Err(_),
                tested_addr,
                ..
            } => Self::AutonatClientTestFailed(tested_addr.clone()),
            autonat::v2::client::Event {
                result: Ok(()),
                tested_addr,
                ..
            } => Self::AutonatClientTestOk(tested_addr.clone()),
        })
    }
}

impl TryFrom<&address_mapper::Event> for Event {
    type Error = ();

    fn try_from(event: &address_mapper::Event) -> Result<Self, Self::Error> {
        Ok(match event {
            address_mapper::Event::AddressMappingFailed(addr) => {
                Self::AddressMappingFailed(addr.clone())
            }
            address_mapper::Event::DefaultGatewayChanged => Self::DefaultGatewayChanged(None),
            address_mapper::Event::LocalAddressChanged(addr) => {
                Self::LocalAddressChanged(addr.clone())
            }
            address_mapper::Event::NewExternalMappedAddress {
                local_address,
                external_address,
            } => Self::NewExternalMappedAddress {
                local_address: local_address.clone(),
                external_address: external_address.clone(),
            },
        })
    }
}

impl TryFrom<FromSwarm<'_>> for Event {
    type Error = ();

    fn try_from(event: FromSwarm<'_>) -> Result<Self, Self::Error> {
        match event {
            FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate { addr }) => {
                Ok(Self::NewExternalAddressCandidate(addr.clone()))
            }
            FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed { addr }) => {
                Ok(Self::ExternalAddressConfirmed(addr.clone()))
            }
            _ => Err(()),
        }
    }
}
