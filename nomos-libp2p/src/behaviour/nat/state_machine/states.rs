use libp2p::Multiaddr;

use super::State;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct Uninitialized;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestIfPublic(pub Multiaddr);
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TryAddressMapping(pub Multiaddr);
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestIfMappedPublic(pub Multiaddr);
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Public(pub Multiaddr);
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MappedPublic(pub Multiaddr);
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Private(pub Multiaddr);

impl From<Uninitialized> for State {
    fn from(value: Uninitialized) -> Self {
        State::Uninitialized(value)
    }
}

impl From<TestIfPublic> for State {
    fn from(value: TestIfPublic) -> Self {
        State::TestIfPublic(value)
    }
}

impl From<TryAddressMapping> for State {
    fn from(value: TryAddressMapping) -> Self {
        State::TryAddressMapping(value)
    }
}

impl From<TestIfMappedPublic> for State {
    fn from(value: TestIfMappedPublic) -> Self {
        State::TestIfMappedPublic(value)
    }
}

impl From<Public> for State {
    fn from(value: Public) -> Self {
        State::Public(value)
    }
}

impl From<MappedPublic> for State {
    fn from(value: MappedPublic) -> Self {
        State::MappedPublic(value)
    }
}

impl From<Private> for State {
    fn from(value: Private) -> Self {
        State::Private(value)
    }
}
