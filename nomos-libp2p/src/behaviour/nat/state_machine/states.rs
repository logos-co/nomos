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
    use std::any::TypeId;

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

    impl PartialEq<Box<dyn OnEvent>> for Box<dyn OnEvent> {
        fn eq(&self, other: &Box<dyn OnEvent>) -> bool {
            self.eq(&other)
        }
    }

    // impl PartialEq<&Box<dyn OnEvent>> for Box<dyn OnEvent> {
    impl PartialEq<&Box<dyn OnEvent>> for Box<dyn OnEvent> {
        fn eq(&self, other: &&Box<dyn OnEvent>) -> bool {
            let lhs = format!("{:?}", self);
            let rhs = format!("{:?}", other);
            eprintln!("Comparing: LHS: {}, RHS: {}", lhs, rhs);
            lhs == rhs

            // let lhs = self as &dyn std::any::Any;
            // let rhs = &**other as &dyn std::any::Any;

            // if downcast_compare::<State<Uninitialized>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<TestIfPublic>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<TryMapAddress>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<TestIfMappedPublic>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<Public>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<MappedPublic>>(lhs, rhs) {
            //     return true;
            // }

            // if downcast_compare::<State<Private>>(lhs, rhs) {
            //     return true;
            // }

            // false
        }
    }

    impl PartialEq<&Box<dyn OnEvent>> for Box<dyn OnEvent> {
        fn eq(&self, other: &&Box<dyn OnEvent>) -> bool {
            let lhs = self as &dyn std::any::Any;
            let rhs = &**other as &dyn std::any::Any;

            if downcast_compare::<State<Uninitialized>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<TestIfPublic>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<TryMapAddress>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<TestIfMappedPublic>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<Public>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<MappedPublic>>(lhs, rhs) {
                return true;
            }

            if downcast_compare::<State<Private>>(lhs, rhs) {
                return true;
            }

            false
        }
    }

    fn downcast_compare<T: PartialEq + Debug + 'static>(
        lhs: &dyn std::any::Any,
        rhs: &dyn std::any::Any,
    ) -> bool {
        eprintln!("L downcast_ref: {:?}", lhs.downcast_ref::<State<T>>());
        eprintln!("R downcast_ref: {:?}", rhs.downcast_ref::<State<T>>());

        let t_typeid = TypeId::of::<T>();
        let lhs_typeid = lhs.type_id();
        let rhs_typeid = rhs.type_id();

        eprintln!(
            "Comparing TypeId: lhs: {:?}, rhs: {:?}, T: {:?}",
            lhs_typeid, rhs_typeid, t_typeid
        );

        if let (Some(lhs_state), Some(rhs_state)) = (
            lhs.downcast_ref::<State<T>>(),
            rhs.downcast_ref::<State<T>>(),
        ) {
            return lhs_state == rhs_state;
        }

        false
    }

    // impl PartialEq<Box<dyn OnEvent>> for Box<dyn OnEventTest> {
    //     fn eq(&self, other: &Box<dyn OnEvent>) -> bool {
    //         self.box_eq(&*other as &dyn std::any::Any)
    //     }
    // }

    // impl PartialEq<&Box<dyn OnEvent>> for Box<dyn OnEventTest> {
    //     fn eq(&self, other: &&Box<dyn OnEvent>) -> bool {
    //         self.box_eq(&**other as &dyn std::any::Any)
    //     }
    // }

    // impl PartialEq<Box<dyn OnEvent>> for Box<dyn OnEventTest> {
    //     fn eq(&self, other: &Box<dyn OnEvent>) -> bool {
    //         self.box_eq(&*other as &dyn std::any::Any)
    //     }
    // }

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
            eprintln!("downcast_ref: {:?}", other.downcast_ref::<Self>());
            other.downcast_ref::<Self>() == Some(self)
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
