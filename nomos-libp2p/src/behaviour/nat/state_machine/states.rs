use super::State;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Uninitialized;
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TestIfPublic<Addr>(pub Addr);
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TryAddressMapping<Addr>(pub Addr);
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TestIfMappedPublic<Addr>(pub Addr);
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Public<Addr>(pub Addr);
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MappedPublic<Addr>(pub Addr);
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Private<Addr>(pub Addr);

impl<Addr> From<Uninitialized> for State<Addr> {
    fn from(value: Uninitialized) -> Self {
        State::Uninitialized(value)
    }
}

impl<Addr> From<TestIfPublic<Addr>> for State<Addr> {
    fn from(value: TestIfPublic<Addr>) -> Self {
        State::TestIfPublic(value)
    }
}

impl<Addr> From<TryAddressMapping<Addr>> for State<Addr> {
    fn from(value: TryAddressMapping<Addr>) -> Self {
        State::TryAddressMapping(value)
    }
}

impl<Addr> From<TestIfMappedPublic<Addr>> for State<Addr> {
    fn from(value: TestIfMappedPublic<Addr>) -> Self {
        State::TestIfMappedPublic(value)
    }
}

impl<Addr> From<Public<Addr>> for State<Addr> {
    fn from(value: Public<Addr>) -> Self {
        State::Public(value)
    }
}

impl<Addr> From<MappedPublic<Addr>> for State<Addr> {
    fn from(value: MappedPublic<Addr>) -> Self {
        State::MappedPublic(value)
    }
}

impl<Addr> From<Private<Addr>> for State<Addr> {
    fn from(value: Private<Addr>) -> Self {
        State::Private(value)
    }
}
