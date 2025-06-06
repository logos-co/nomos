use libp2p::{autonat, swarm::FromSwarm};

use crate::behaviour::nat::address_mapper;

pub(super) enum TestIfPublicEvent {
    ExternalAddressConfirmed,
    AutonatClientTestFailed,
}

pub(super) enum TryAddressMappingEvent {
    NewExternalMappedAddress,
    AddressMappingFailed,
}

pub(super) enum TestIfMappedPublicEvent {
    ExternalAddressConfirmed,
    AutonatClientTestFailed,
}

pub(super) enum PublicEvent {
    ExternalAddressConfirmed,
    AutonatClientTestOk,
    AutonatClientTestFailed,
}

pub(super) enum MappedPublicEvent {
    ExternalAddressConfirmed,
    AutonatClientTestOk,
    AutonatClientTestFailed,
}

pub(super) enum PrivateEvent {
    LocalAddressChanged,
    DefaultGatewayChanged,
}

impl TryFrom<Event> for TestIfPublicEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed => Ok(TestIfPublicEvent::ExternalAddressConfirmed),
            Event::AutonatClientTestFailed => Ok(TestIfPublicEvent::AutonatClientTestFailed),
            _ => Err(()),
        }
    }
}

impl TryFrom<Event> for TryAddressMappingEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::_NewExternalMappedAddress => {
                Ok(TryAddressMappingEvent::NewExternalMappedAddress)
            }
            Event::AddressMappingFailed => Ok(TryAddressMappingEvent::AddressMappingFailed),
            _ => Err(()),
        }
    }
}

impl TryFrom<Event> for TestIfMappedPublicEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed => {
                Ok(TestIfMappedPublicEvent::ExternalAddressConfirmed)
            }
            Event::AutonatClientTestFailed => Ok(TestIfMappedPublicEvent::AutonatClientTestFailed),
            _ => Err(()),
        }
    }
}

impl TryFrom<Event> for PublicEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed => Ok(PublicEvent::ExternalAddressConfirmed),
            Event::AutonatClientTestOk => Ok(PublicEvent::AutonatClientTestOk),
            Event::AutonatClientTestFailed => Ok(PublicEvent::AutonatClientTestFailed),
            _ => Err(()),
        }
    }
}

impl TryFrom<Event> for MappedPublicEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::ExternalAddressConfirmed => Ok(MappedPublicEvent::ExternalAddressConfirmed),
            Event::AutonatClientTestOk => Ok(MappedPublicEvent::AutonatClientTestOk),
            Event::AutonatClientTestFailed => Ok(MappedPublicEvent::AutonatClientTestFailed),
            _ => Err(()),
        }
    }
}

impl TryFrom<Event> for PrivateEvent {
    type Error = ();

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::_LocalAddressChanged => Ok(PrivateEvent::LocalAddressChanged),
            Event::_DefaultGatewayChanged => Ok(PrivateEvent::DefaultGatewayChanged),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Event {
    AutonatClientTestOk,
    AutonatClientTestFailed,
    AddressMappingFailed,
    _DefaultGatewayChanged,
    ExternalAddressConfirmed,
    _LocalAddressChanged,
    _NewExternalMappedAddress,
}

impl TryFrom<&FromSwarm<'_>> for Event {
    type Error = ();

    fn try_from(event: &FromSwarm<'_>) -> Result<Self, Self::Error> {
        match event {
            FromSwarm::ExternalAddrConfirmed(_) => Ok(Event::ExternalAddressConfirmed),
            _ => Err(()),
        }
    }
}

impl TryFrom<&autonat::v2::client::Event> for Event {
    type Error = ();

    fn try_from(event: &autonat::v2::client::Event) -> Result<Self, Self::Error> {
        match event {
            autonat::v2::client::Event { result: Err(_), .. } => Ok(Event::AutonatClientTestFailed),
            autonat::v2::client::Event { result: Ok(_), .. } => Ok(Event::AutonatClientTestOk),
        }
    }
}

impl TryFrom<&address_mapper::Event> for Event {
    type Error = ();

    fn try_from(event: &address_mapper::Event) -> Result<Self, Self::Error> {
        let address_mapper::Event::AddressMappingFailed(address) = event;
        Ok(Event::AddressMappingFailed)
    }
}
