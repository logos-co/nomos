use libp2p::{
    autonat,
    swarm::{behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate},
    Multiaddr,
};

use crate::behaviour::nat::address_mapper;

pub(crate) enum UninitializedEvent {
    NewExternalAddressCandidate(Multiaddr),
}

pub(crate) enum TestIfPublicEvent {
    ExternalAddressConfirmed(Multiaddr),
    AutonatClientTestFailed(Multiaddr),
}

pub(crate) enum TryAddressMappingEvent {
    _NewExternalMappedAddress(Multiaddr),
    AddressMappingFailed(Multiaddr),
}

pub(crate) enum TestIfMappedPublicEvent {
    ExternalAddressConfirmed(Multiaddr),
    AutonatClientTestFailed(Multiaddr),
}

pub(crate) enum PublicEvent {
    ExternalAddressConfirmed(Multiaddr),
    AutonatClientTestOk(Multiaddr),
    AutonatClientTestFailed(Multiaddr),
}

pub(crate) enum MappedPublicEvent {
    ExternalAddressConfirmed(Multiaddr),
    AutonatClientTestOk(Multiaddr),
    AutonatClientTestFailed(Multiaddr),
}

#[derive(Debug)]
pub(crate) enum PrivateEvent {
    // TODO impl TryFrom<AddressMapperBehaviour::Event> when it's available
    _LocalAddressChanged,
    // TODO impl TryFrom<AddressMapperBehaviour::Event> when it's available
    _DefaultGatewayChanged,
}

macro_rules! impl_try_from {
    ($in_type:ty, $arg_name:ident, $out_type:ty, $block:block) => {
        impl TryFrom<$in_type> for $out_type {
            type Error = ();

            fn try_from($arg_name: $in_type) -> Result<Self, Self::Error> {
                $block
            }
        }
    };
}

/// For each `$out_type`, implement `TryFrom<$in_type>`, using code block
/// `$block`. Caller of this macro must use `Self` to indicate the `$out_type`
/// in `$block`.
///
/// Thanks to this macro we can jump straight from a libp2p event to a
/// type-safe, state-specific event.
macro_rules! impl_try_from_foreach {
    ($in_type:ty, $arg_name:ident, [$($out_type:ty),+ $(,)?], $block:block) => {
        $(impl_try_from!($in_type, $arg_name, $out_type, $block);)+
    };
}

#[cfg(test)]
pub(super) use {impl_try_from, impl_try_from_foreach};

impl_try_from_foreach!(FromSwarm<'_>, event, [UninitializedEvent], {
    match event {
        FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate { addr }) => {
            Ok(Self::NewExternalAddressCandidate(addr.clone()))
        }
        _ => Err(()),
    }
});

impl_try_from_foreach!(
    FromSwarm<'_>,
    event,
    [
        TestIfPublicEvent,
        TestIfMappedPublicEvent,
        PublicEvent,
        MappedPublicEvent,
    ],
    {
        match event {
            FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed { addr }) => {
                Ok(Self::ExternalAddressConfirmed(addr.clone()))
            }
            _ => Err(()),
        }
    }
);

impl_try_from_foreach!(
    FromSwarm<'_>,
    _event,
    [TryAddressMappingEvent, PrivateEvent],
    { Err(()) }
);

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    event,
    [TestIfPublicEvent, TestIfMappedPublicEvent],
    {
        match event {
            autonat::v2::client::Event {
                result: Err(_),
                tested_addr,
                ..
            } => Ok(Self::AutonatClientTestFailed(tested_addr.clone())),
            _ => Err(()),
        }
    }
);

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    event,
    [PublicEvent, MappedPublicEvent],
    {
        match event {
            autonat::v2::client::Event {
                result: Err(_),
                tested_addr,
                ..
            } => Ok(Self::AutonatClientTestFailed(tested_addr.clone())),
            autonat::v2::client::Event {
                result: Ok(_),
                tested_addr,
                ..
            } => Ok(Self::AutonatClientTestOk(tested_addr.clone())),
        }
    }
);

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    _event,
    [UninitializedEvent, TryAddressMappingEvent, PrivateEvent],
    { Err(()) }
);

impl_try_from_foreach!(&address_mapper::Event, event, [TryAddressMappingEvent], {
    match event {
        address_mapper::Event::AddressMappingFailed(addr) => {
            Ok(Self::AddressMappingFailed(addr.clone()))
        }
        address_mapper::Event::_NewExternalMappedAddress(addr) => {
            Ok(Self::_NewExternalMappedAddress(addr.clone()))
        }
    }
});

impl_try_from_foreach!(
    &address_mapper::Event,
    _event,
    [
        UninitializedEvent,
        TestIfPublicEvent,
        TestIfMappedPublicEvent,
        PublicEvent,
        MappedPublicEvent,
        PrivateEvent
    ],
    { Err(()) }
);
