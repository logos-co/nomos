use libp2p::{
    autonat,
    swarm::{behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate},
    Multiaddr,
};

use crate::behaviour::nat::address_mapper;

pub(super) enum UninitializedEvent<Addr> {
    NewExternalAddressCandidate(Addr),
}

pub(super) enum TestIfPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestFailed(Addr),
}

pub(super) enum TryAddressMappingEvent<Addr> {
    NewExternalMappedAddress(Addr),
    AddressMappingFailed(Addr),
}

pub(super) enum TestIfMappedPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestFailed(Addr),
}

pub(super) enum PublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestOk(Addr),
    AutonatClientTestFailed(Addr),
}

pub(super) enum MappedPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestOk(Addr),
    AutonatClientTestFailed(Addr),
}

pub(super) enum PrivateEvent {
    LocalAddressChanged,
    DefaultGatewayChanged,
}

impl<Addr> TryFrom<Event<Addr>> for UninitializedEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::NewExternalAddressCandidate(addr) => {
                Ok(UninitializedEvent::NewExternalAddressCandidate(addr))
            }
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for TestIfPublicEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed(addr) => {
                Ok(TestIfPublicEvent::ExternalAddressConfirmed(addr))
            }
            Event::AutonatClientTestFailed(addr) => {
                Ok(TestIfPublicEvent::AutonatClientTestFailed(addr))
            }
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for TryAddressMappingEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::_NewExternalMappedAddress(addr) => {
                Ok(TryAddressMappingEvent::NewExternalMappedAddress(addr))
            }
            Event::AddressMappingFailed(addr) => {
                Ok(TryAddressMappingEvent::AddressMappingFailed(addr))
            }
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for TestIfMappedPublicEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed(addr) => {
                Ok(TestIfMappedPublicEvent::ExternalAddressConfirmed(addr))
            }
            Event::AutonatClientTestFailed(addr) => {
                Ok(TestIfMappedPublicEvent::AutonatClientTestFailed(addr))
            }
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for PublicEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed(addr) => {
                Ok(PublicEvent::ExternalAddressConfirmed(addr))
            }
            Event::AutonatClientTestOk(addr) => Ok(PublicEvent::AutonatClientTestOk(addr)),
            Event::AutonatClientTestFailed(addr) => Ok(PublicEvent::AutonatClientTestFailed(addr)),
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for MappedPublicEvent<Addr> {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed(addr) => {
                Ok(MappedPublicEvent::ExternalAddressConfirmed(addr))
            }
            Event::AutonatClientTestOk(addr) => Ok(MappedPublicEvent::AutonatClientTestOk(addr)),
            Event::AutonatClientTestFailed(addr) => {
                Ok(MappedPublicEvent::AutonatClientTestFailed(addr))
            }
            _ => Err(()),
        }
    }
}

impl<Addr> TryFrom<Event<Addr>> for PrivateEvent {
    type Error = ();

    fn try_from(event: Event<Addr>) -> Result<Self, Self::Error> {
        match event {
            Event::_LocalAddressChanged => Ok(PrivateEvent::LocalAddressChanged),
            Event::_DefaultGatewayChanged => Ok(PrivateEvent::DefaultGatewayChanged),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Event<Addr> {
    AutonatClientTestOk(Addr),
    AutonatClientTestFailed(Addr),
    AddressMappingFailed(Addr),
    _DefaultGatewayChanged,
    ExternalAddressConfirmed(Addr),
    _LocalAddressChanged,
    NewExternalAddressCandidate(Addr),
    _NewExternalMappedAddress(Addr),
}

impl TryFrom<FromSwarm<'_>> for Event<Multiaddr> {
    type Error = ();

    fn try_from(event: FromSwarm<'_>) -> Result<Self, Self::Error> {
        match event {
            FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate { addr }) => {
                Ok(Event::NewExternalAddressCandidate(addr.clone()))
            }
            FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed { addr }) => {
                Ok(Event::ExternalAddressConfirmed(addr.clone()))
            }
            _ => Err(()),
        }
    }
}

impl TryFrom<&autonat::v2::client::Event> for Event<Multiaddr> {
    type Error = ();

    fn try_from(event: &autonat::v2::client::Event) -> Result<Self, Self::Error> {
        match event {
            autonat::v2::client::Event {
                result: Err(_),
                tested_addr,
                ..
            } => Ok(Event::AutonatClientTestFailed(tested_addr.clone())),
            autonat::v2::client::Event {
                result: Ok(_),
                tested_addr,
                ..
            } => Ok(Event::AutonatClientTestOk(tested_addr.clone())),
        }
    }
}

impl TryFrom<&address_mapper::Event> for Event<Multiaddr> {
    type Error = ();

    fn try_from(event: &address_mapper::Event) -> Result<Self, Self::Error> {
        let address_mapper::Event::AddressMappingFailed(address) = event;
        Ok(Event::AddressMappingFailed(address.clone()))
    }
}
