use std::fmt::Debug;

use libp2p::Multiaddr;

impl super::State<Uninitialized> {
    pub const fn new() -> Self {
        Self {
            state: Uninitialized(()),
        }
    }
}

/// Represents the initial state of the state machine. There is no known
/// external address at this point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uninitialized(
    // Intentionally private so that this state cannot be constructed outside this module
    (),
);

impl Uninitialized {
    #[expect(
        clippy::unused_self,
        reason = "The aim of the pattern is to consume self."
    )]
    pub const fn into_test_if_public(self, addr_to_test: Multiaddr) -> TestIfPublic {
        TestIfPublic { addr_to_test }
    }
}

/// We are aware of an external address candidate and we want to test if it is
/// public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIfPublic {
    addr_to_test: Multiaddr,
}

impl TestIfPublic {
    pub fn into_public(self) -> Public {
        let Self { addr_to_test } = self;
        Public { addr: addr_to_test }
    }

    pub fn into_try_map_address(self) -> TryMapAddress {
        let Self { addr_to_test } = self;
        TryMapAddress {
            addr_to_map: addr_to_test,
        }
    }

    pub const fn addr_to_test(&self) -> &Multiaddr {
        &self.addr_to_test
    }
}

/// The address of the node is known, but it is not publicly reachable. We want
/// to map it to a publicly reachable address one the NAT-box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryMapAddress {
    addr_to_map: Multiaddr,
}

impl TryMapAddress {
    #[expect(
        clippy::unused_self,
        reason = "The aim of the pattern is to consume self."
    )]
    pub fn into_test_if_mapped_public(self, new_external_addr: Multiaddr) -> TestIfMappedPublic {
        TestIfMappedPublic {
            addr_to_test: new_external_addr,
        }
    }

    pub fn into_private(self) -> Private {
        let Self { addr_to_map } = self;
        Private { addr: addr_to_map }
    }

    pub const fn addr_to_map(&self) -> &Multiaddr {
        &self.addr_to_map
    }
}

/// The address of the node is known, it is not publicly reachable, but it
/// has been mapped to Internet-facing address on the NAT-box. We want to
/// test if the outside address of the NAT-box is indeed publicly reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIfMappedPublic {
    addr_to_test: Multiaddr,
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

    pub const fn addr_to_test(&self) -> &Multiaddr {
        &self.addr_to_test
    }
}

/// The address of the node is known, it is publicly reachable, and it has been
/// confirmed by the `autonat` client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Public {
    addr: Multiaddr,
}

impl Public {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub const fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

/// The address of the node is known, it is not publicly reachable, but it has
/// been mapped to a publicly reachable address on the NAT-box, which has been
/// confirmed by the `autonat` client to be publicly reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedPublic {
    addr: Multiaddr,
}

impl MappedPublic {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub fn into_try_map_address(self) -> TryMapAddress {
        let Self { addr } = self;
        TryMapAddress { addr_to_map: addr }
    }

    pub const fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

/// The address of the node is known, it is not publicly reachable, and it has
/// not been successfully mapped to a publicly reachable address on the NAT-box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Private {
    addr: Multiaddr,
}

impl Private {
    #[expect(
        clippy::unused_self,
        reason = "The aim of the pattern is to consume self."
    )]
    pub fn into_test_if_public(self, new_addr: Multiaddr) -> TestIfPublic {
        TestIfPublic {
            addr_to_test: new_addr,
        }
    }

    pub const fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

#[cfg(test)]
pub mod test_utils {
    use super::*;
    use crate::behaviour::nat::state_machine::{OnEvent, State};

    impl Uninitialized {
        pub(crate) fn for_test() -> Box<dyn OnEvent> {
            Box::new(State::<Self>::new())
        }
    }

    impl TestIfPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr_to_test: addr },
            })
        }
    }

    impl TryMapAddress {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr_to_map: addr },
            })
        }
    }

    impl TestIfMappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr_to_test: addr },
            })
        }
    }

    impl Public {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }

    impl MappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }

    impl Private {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEvent> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }
}
