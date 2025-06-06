use libp2p::{
    autonat,
    swarm::{behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate},
    Multiaddr,
};

use crate::behaviour::nat::address_mapper;

pub(crate) enum UninitializedEvent<Addr> {
    NewExternalAddressCandidate(Addr),
}

pub(crate) enum TestIfPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestFailed(Addr),
}

pub(crate) enum TryAddressMappingEvent<Addr> {
    // TODO impl TryFrom<AddressMapperBehaviour::Event> when it's available
    _NewExternalMappedAddress(Addr),
    AddressMappingFailed(Addr),
}

pub(crate) enum TestIfMappedPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestFailed(Addr),
}

pub(crate) enum PublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestOk(Addr),
    AutonatClientTestFailed(Addr),
}

pub(crate) enum MappedPublicEvent<Addr> {
    ExternalAddressConfirmed(Addr),
    AutonatClientTestOk(Addr),
    AutonatClientTestFailed(Addr),
}

pub(crate) enum PrivateEvent {
    // TODO impl TryFrom<???::Event> when it's available
    _LocalAddressChanged,
    // TODO impl TryFrom<AddressMapperBehaviour::Event> when it's available
    _DefaultGatewayChanged,
}

/*
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
*/

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

impl_try_from_foreach!(FromSwarm<'_>, event, [UninitializedEvent<Multiaddr>], {
    match event {
        FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate { addr }) => {
            Ok(Self::NewExternalAddressCandidate(addr.clone()))
        }
        _ => Err(()),
    }
});

impl_try_from_foreach!(FromSwarm<'_>, event, [
    TestIfPublicEvent<Multiaddr>,
    TestIfMappedPublicEvent<Multiaddr>,
    PublicEvent<Multiaddr>,
    MappedPublicEvent<Multiaddr>,
    ], {
    match event {
        FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed { addr }) => {
            Ok(Self::ExternalAddressConfirmed(addr.clone()))
        }
        _ => Err(()),
    }
});

impl_try_from_foreach!(FromSwarm<'_>, _event, [TryAddressMappingEvent<Multiaddr>, PrivateEvent], {
    Err(())
});

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    event,
    [TestIfPublicEvent<Multiaddr>, TestIfMappedPublicEvent<Multiaddr>],
    {
    match event {
        autonat::v2::client::Event {
            result: Err(_),
            tested_addr,
            ..
        } => Ok(Self::AutonatClientTestFailed(tested_addr.clone())),
        _ => Err(())
    }
});

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    event,
    [PublicEvent<Multiaddr>, MappedPublicEvent<Multiaddr>],
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
});

impl_try_from_foreach!(
    &autonat::v2::client::Event,
    _event,
    [UninitializedEvent<Multiaddr>, TryAddressMappingEvent<Multiaddr>, PrivateEvent],
    { Err(()) }
);

impl_try_from_foreach!(
    &address_mapper::Event,
    event,
    [TryAddressMappingEvent<Multiaddr>],
    {
        let address_mapper::Event::AddressMappingFailed(address) = event;
        Ok(Self::AddressMappingFailed(address.clone()))
    }
);

impl_try_from_foreach!(
    &address_mapper::Event,
    _event,
    [
        UninitializedEvent<Multiaddr>,
        TestIfPublicEvent<Multiaddr>,
        TestIfMappedPublicEvent<Multiaddr>,
        PublicEvent<Multiaddr>,
        MappedPublicEvent<Multiaddr>,
        PrivateEvent
    ],
    { Err(()) }
);
