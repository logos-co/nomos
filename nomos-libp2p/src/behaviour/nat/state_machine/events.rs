use libp2p::{autonat, swarm::FromSwarm};

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
            Event::NewExternalMappedAddress => Ok(TryAddressMappingEvent::NewExternalMappedAddress),
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
            Event::LocalAddressChanged => Ok(PrivateEvent::LocalAddressChanged),
            Event::DefaultGatewayChanged => Ok(PrivateEvent::DefaultGatewayChanged),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Event {
    AutonatClientTestOk,
    AutonatClientTestFailed,
    AddressMappingFailed,
    DefaultGatewayChanged,
    ExternalAddressConfirmed,
    LocalAddressChanged,
    NewExternalMappedAddress,
}

/// TODO improve this comment or move it to a better place
///
/// autonat::v2::Client successful flow:
/// 1. FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed(addr))
/// 2. autonat::v2::client::Event { result == Ok(()), tested_addr == addr, .. }
///
/// So we need to intercept the first event only and we can happily ignore the
/// second one.
impl TryFrom<&FromSwarm<'_>> for Event {
    type Error = ();

    fn try_from(event: &FromSwarm<'_>) -> Result<Self, Self::Error> {
        match event {
            FromSwarm::ExternalAddrConfirmed(_) => Ok(Event::ExternalAddressConfirmed),
            _ => Err(()),
        }
    }
}

/// TODO improve this comment or move it to a better place
///
/// autonat::v2::Client failure flow:
/// 1. autonat::v2::client::Event { result == Err(..), tested_addr, .. }
///
/// So we need to intercept the first event only and we can happily ignore the
/// second one.
impl TryFrom<&autonat::v2::client::Event> for Event {
    type Error = ();

    fn try_from(event: &autonat::v2::client::Event) -> Result<Self, Self::Error> {
        match event {
            autonat::v2::client::Event { result: Err(_), .. } => Ok(Event::AutonatClientTestFailed),
            autonat::v2::client::Event { result: Ok(_), .. } => Ok(Event::AutonatClientTestOk),
        }
    }
}
