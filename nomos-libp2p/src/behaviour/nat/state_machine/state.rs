use std::{fmt::Debug, marker::PhantomData};

use libp2p::Multiaddr;

impl<E: Debug> super::State<Uninitialized, E> {
    pub fn new() -> Self {
        Self {
            state: Uninitialized(()),
            _phantom_event: PhantomData,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Uninitialized(
    // Intentionally private so that this state cannot be constructed outside this module
    (),
);

#[derive(Debug, Clone, PartialEq)]
pub struct TestIfPublic {
    addr_to_test: Multiaddr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TryMapAddress {
    addr_to_map: Multiaddr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestIfMappedPublic {
    addr_to_test: Multiaddr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Public {
    addr: Multiaddr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MappedPublic {
    addr: Multiaddr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Private {
    addr: Multiaddr,
}

impl Uninitialized {
    pub fn into_test_if_public(self, addr_to_test: Multiaddr) -> TestIfPublic {
        TestIfPublic { addr_to_test }
    }
}

impl TestIfPublic {
    pub fn into_public(self) -> Public {
        let Self { addr_to_test } = self;
        Public { addr: addr_to_test }
    }

    pub fn into_try_address_mapping(self) -> TryMapAddress {
        let Self { addr_to_test } = self;
        TryMapAddress {
            addr_to_map: addr_to_test,
        }
    }

    pub fn addr_to_test(&self) -> &Multiaddr {
        &self.addr_to_test
    }
}

impl TryMapAddress {
    pub fn into_test_if_mapped_public(self, new_external_addr: Multiaddr) -> TestIfMappedPublic {
        TestIfMappedPublic {
            addr_to_test: new_external_addr,
        }
    }

    pub fn into_private(self) -> Private {
        let Self { addr_to_map } = self;
        Private { addr: addr_to_map }
    }

    pub fn addr_to_map(&self) -> &Multiaddr {
        &self.addr_to_map
    }
}

impl TestIfMappedPublic {
    pub fn into_mapped_public(self) -> MappedPublic {
        let Self { addr_to_test } = self;
        MappedPublic { addr: addr_to_test }
    }

    pub fn into_private(self) -> Private {
        let Self { addr_to_test } = self;
        Private { addr: addr_to_test }
    }

    pub fn addr_to_test(&self) -> &Multiaddr {
        &self.addr_to_test
    }
}

impl Public {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

impl MappedPublic {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

impl Private {
    pub fn into_test_if_public(self, new_addr: Multiaddr) -> TestIfPublic {
        TestIfPublic {
            addr_to_test: new_addr,
        }
    }

    pub fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use libp2p::Multiaddr;

    use super::*;
    use crate::behaviour::nat::state_machine::{event::Event, OnEvent, State};

    impl Uninitialized {
        pub(crate) fn for_test() -> Box<dyn OnEvent<Event>> {
            Box::new(State::<Uninitialized, Event>::new())
        }
    }

    impl TestIfPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<TestIfPublic, Event> {
                state: TestIfPublic { addr_to_test: addr },
                _phantom_event: PhantomData,
            })
        }
    }

    impl TryMapAddress {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<TryMapAddress, Event> {
                state: TryMapAddress { addr_to_map: addr },
                _phantom_event: PhantomData,
            })
        }
    }

    impl TestIfMappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<TestIfMappedPublic, Event> {
                state: TestIfMappedPublic { addr_to_test: addr },
                _phantom_event: PhantomData,
            })
        }
    }

    impl Public {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<Public, Event> {
                state: Public { addr },
                _phantom_event: PhantomData,
            })
        }
    }

    impl MappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<MappedPublic, Event> {
                state: MappedPublic { addr },
                _phantom_event: PhantomData,
            })
        }
    }

    impl Private {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent<Event>> {
            Box::new(State::<Private, Event> {
                state: Private { addr },
                _phantom_event: PhantomData,
            })
        }
    }
}
