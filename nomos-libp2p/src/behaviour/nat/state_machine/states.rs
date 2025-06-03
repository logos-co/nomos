use super::State;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TestIfPublic;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TryAddressMapping;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TestIfMappedPublic;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Public;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MappedPublic;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Private;

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
