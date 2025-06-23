use std::fmt::Debug;

use libp2p::Multiaddr;

impl super::State<Uninitialized> {
    pub const fn new() -> Self {
        Self {
            state: Uninitialized(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uninitialized(
    // Intentionally private so that this state cannot be constructed outside this module
    (),
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIfPublic {
    addr_to_test: Multiaddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryMapAddress {
    addr_to_map: Multiaddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIfMappedPublic {
    addr_to_test: Multiaddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Public {
    addr: Multiaddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedPublic {
    addr: Multiaddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Private {
    addr: Multiaddr,
}

impl Uninitialized {
    #[expect(
        clippy::unused_self,
        reason = "The aim of the pattern is to consume self."
    )]
    pub const fn into_test_if_public(self, addr_to_test: Multiaddr) -> TestIfPublic {
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

    pub const fn addr_to_test(&self) -> &Multiaddr {
        &self.addr_to_test
    }
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

impl Public {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub const fn addr(&self) -> &Multiaddr {
        &self.addr
    }
}

impl MappedPublic {
    pub fn into_test_if_public(self) -> TestIfPublic {
        let Self { addr } = self;
        TestIfPublic { addr_to_test: addr }
    }

    pub const fn addr(&self) -> &Multiaddr {
        &self.addr
    }
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

    pub(crate) trait OnEventTest: OnEvent + DynPartialEq {}

    impl OnEventTest for State<Uninitialized> {}
    impl OnEventTest for State<TestIfPublic> {}
    impl OnEventTest for State<TryMapAddress> {}
    impl OnEventTest for State<TestIfMappedPublic> {}
    impl OnEventTest for State<Public> {}
    impl OnEventTest for State<MappedPublic> {}
    impl OnEventTest for State<Private> {}

    // PartialEq<&Box<dyn state_machine::OnEvent>>` is not implemented for `Box<dyn
    // OnEventTest>`

    impl PartialEq<&Box<dyn OnEvent>> for Box<dyn OnEventTest> {
        fn eq(&self, other: &&Box<dyn OnEvent>) -> bool {
            self.box_eq(&**other as &dyn std::any::Any)
        }
    }

    impl PartialEq<Box<dyn OnEvent>> for Box<dyn OnEventTest> {
        fn eq(&self, other: &Box<dyn OnEvent>) -> bool {
            self.box_eq(&*other as &dyn std::any::Any)
        }
    }

    // impl PartialEq<&'static dyn OnEvent> for &'static dyn OnEventTest {
    //     fn eq(&self, other: &&'static dyn OnEvent) -> bool {
    //         self.box_eq(other as &dyn std::any::Any)
    //     }
    // }

    // impl PartialEq<&'static dyn OnEventTest> for &'static dyn OnEvent {
    //     fn eq(&self, other: &&'static dyn OnEventTest) -> bool {
    //         other.box_eq(self as &dyn std::any::Any)
    //     }
    // }

    pub trait DynPartialEq {
        fn box_eq(&self, other: &dyn std::any::Any) -> bool;
        fn as_any(&self) -> &dyn std::any::Any;
    }

    impl<S: PartialEq + 'static + Debug> DynPartialEq for State<S> {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn box_eq(&self, other: &dyn core::any::Any) -> bool {
            let other = unsafe { &*(other as *const dyn std::any::Any as *const Self) };
            eprintln!("self {:#?} other {:#?}", self, other);

            // other.downcast_ref::<Self>() == Some(self)

            other == self
        }
    }

    impl PartialEq for Box<dyn OnEventTest> {
        fn eq(&self, other: &Self) -> bool {
            self.box_eq(other.as_any())
        }
    }

    impl PartialEq<&Self> for Box<dyn OnEventTest> {
        fn eq(&self, other: &&Self) -> bool {
            self.box_eq(other.as_any())
        }
    }

    impl Uninitialized {
        pub(crate) fn for_test() -> Box<dyn OnEventTest> {
            Box::new(State::<Self>::new())
        }
    }

    impl TestIfPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr_to_test: addr },
            })
        }
    }

    impl TryMapAddress {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr_to_map: addr },
            })
        }
    }

    impl TestIfMappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr_to_test: addr },
            })
        }
    }

    impl Public {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }

    impl MappedPublic {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }

    impl Private {
        pub(crate) fn for_test(addr: Multiaddr) -> Box<dyn OnEventTest> {
            Box::new(State::<Self> {
                state: Self { addr },
            })
        }
    }
}
