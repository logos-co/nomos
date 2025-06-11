use libp2p::Multiaddr;

use super::StateWrapper;
use crate::behaviour::nat::state_machine::State;

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

impl From<State<Uninitialized>> for StateWrapper {
    fn from(value: State<Uninitialized>) -> Self {
        StateWrapper::Uninitialized(value)
    }
}

impl From<State<TestIfPublic>> for StateWrapper {
    fn from(value: State<TestIfPublic>) -> Self {
        StateWrapper::TestIfPublic(value)
    }
}

impl From<State<TryAddressMapping>> for StateWrapper {
    fn from(value: State<TryAddressMapping>) -> Self {
        StateWrapper::TryAddressMapping(value)
    }
}
/*
impl From<TestIfMappedPublic> for StateWrapper {
    fn from(value: TestIfMappedPublic) -> Self {
        StateWrapper::TestIfMappedPublic(value)
    }
}
*/
impl From<State<Public>> for StateWrapper {
    fn from(value: State<Public>) -> Self {
        StateWrapper::Public(value)
    }
}
/*
impl From<MappedPublic> for StateWrapper {
    fn from(value: MappedPublic) -> Self {
        StateWrapper::MappedPublic(value)
    }
}

impl From<Private> for StateWrapper {
    fn from(value: Private) -> Self {
        StateWrapper::Private(value)
    }
}
*/
