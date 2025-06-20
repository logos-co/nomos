use std::fmt::Debug;

use libp2p::Multiaddr;
use tokio::sync::mpsc::UnboundedSender;

mod event;
mod state;

use event::Event;
use state::{
    MappedPublic, Private, Public, TestIfMappedPublic, TestIfPublic, TryMapAddress, Uninitialized,
};

/// Commands that can be issued by the state machine to `NatBehaviour`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    ScheduleAutonatClientTest(Multiaddr),
    MapAddress(Multiaddr),
    NewExternalAddrCandidate(Multiaddr),
}

#[derive(Debug)]
pub struct StateMachine {
    inner: Option<Box<dyn OnEvent>>,
    command_tx: CommandTx,
}

impl StateMachine {
    pub fn new(command_tx: UnboundedSender<Command>) -> Self {
        Self {
            inner: Some(Box::new(State::<Uninitialized>::new())),
            command_tx: command_tx.into(),
        }
    }

    pub fn on_event<E>(&mut self, event: E)
    where
        E: TryInto<Event, Error = ()>,
    {
        let current_state = self.inner.take().expect("State to be Some");

        match event.try_into() {
            Err(()) => {
                // Ignore unrecognized events
                self.inner = Some(current_state);
            }
            Ok(event) => {
                self.inner = Some(current_state.on_event(event, &self.command_tx));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct State<S> {
    pub state: S,
}

#[derive(Debug, Clone)]
struct CommandTx {
    tx: UnboundedSender<Command>,
}

impl From<UnboundedSender<Command>> for CommandTx {
    fn from(tx: UnboundedSender<Command>) -> Self {
        Self { tx }
    }
}

impl CommandTx {
    fn force_send(&self, command: Command) {
        self.tx.send(command).expect("Channel not to be closed");
    }
}

#[cfg(not(test))]
trait OnEvent: Debug + Send {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent>;
}

#[cfg(test)]
trait OnEvent: Debug + Send + DynPartialEq {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent>;
}

#[cfg(test)]
trait DynPartialEq {
    fn box_eq(&self, other: &dyn std::any::Any) -> bool;
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<S> State<S> {
    fn boxed<T, C>(self, next_state_ctor: C) -> Box<State<T>>
    where
        C: FnOnce(S) -> T,
    {
        let Self { state, .. } = self;
        Box::new(State {
            state: next_state_ctor(state),
        })
    }
}

impl OnEvent for State<Uninitialized> {
    fn on_event(self: Box<Self>, event: Event, _: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::NewExternalAddressCandidate(addr) => {
                self.boxed(|state| state.into_test_if_public(addr))
            }
            _ => self,
        }
    }
}

impl OnEvent for State<TestIfPublic> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::ExternalAddressConfirmed(addr) if self.state.addr_to_test() == &addr => {
                command_tx.force_send(Command::ScheduleAutonatClientTest(addr));
                self.boxed(TestIfPublic::into_public)
            }
            Event::AutonatClientTestFailed(addr) if self.state.addr_to_test() == &addr => {
                command_tx.force_send(Command::MapAddress(addr));
                self.boxed(TestIfPublic::into_try_address_mapping)
            }
            Event::ExternalAddressConfirmed(addr) | Event::AutonatClientTestFailed(addr) => {
                panic!(
                    "State<TestIfPublic>: Autonat client reported address {}, but {} was expected",
                    addr,
                    self.state.addr_to_test(),
                );
            }
            _ => self,
        }
    }
}

impl OnEvent for State<TryMapAddress> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::NewExternalMappedAddress(addr) => {
                command_tx.force_send(Command::NewExternalAddrCandidate(addr.clone()));
                self.boxed(|state| state.into_test_if_mapped_public(addr))
            }
            Event::AddressMappingFailed(addr) if self.state.addr_to_map() == &addr => {
                self.boxed(TryMapAddress::into_private)
            }
            Event::AddressMappingFailed(addr) => {
                panic!(
                    "State<TryMapAddress>: Address mapper reported failure for address {}, but {} was expected",
                    addr,
                    self.state.addr_to_map(),
                );
            }
            _ => self,
        }
    }
}

impl OnEvent for State<TestIfMappedPublic> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::ExternalAddressConfirmed(addr) if self.state.addr_to_test() == &addr => {
                command_tx.force_send(Command::ScheduleAutonatClientTest(addr));
                self.boxed(TestIfMappedPublic::into_mapped_public)
            }
            Event::AutonatClientTestFailed(addr) if self.state.addr_to_test() == &addr => {
                self.boxed(TestIfMappedPublic::into_private)
            }
            Event::ExternalAddressConfirmed(addr) | Event::AutonatClientTestFailed(addr) => {
                panic!(
                    "State<TestIfMappedPublic>: Autonat client reported address {}, but {} was expected",
                    addr,
                    self.state.addr_to_test(),
                );
            }
            _ => self,
        }
    }
}

impl OnEvent for State<Public> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::ExternalAddressConfirmed(addr) | Event::AutonatClientTestOk(addr)
                if self.state.addr() == &addr =>
            {
                command_tx.force_send(Command::ScheduleAutonatClientTest(addr));
                self
            }
            Event::AutonatClientTestFailed(addr) if self.state.addr() == &addr => {
                self.boxed(Public::into_test_if_public)
            }
            Event::ExternalAddressConfirmed(addr) => {
                panic!(
                    "State<Public>: Swarm confirmed external address {}, but {} was expected",
                    addr,
                    self.state.addr(),
                );
            }
            Event::AutonatClientTestOk(addr) | Event::AutonatClientTestFailed(addr) => {
                panic!(
                    "State<Public>: Autonat client reported address {}, but {} was expected",
                    addr,
                    self.state.addr(),
                );
            }
            _ => self,
        }
    }
}

impl OnEvent for State<MappedPublic> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::ExternalAddressConfirmed(addr) | Event::AutonatClientTestOk(addr)
                if self.state.addr() == &addr =>
            {
                command_tx.force_send(Command::ScheduleAutonatClientTest(addr));
                self
            }
            Event::AutonatClientTestFailed(addr) if self.state.addr() == &addr => {
                self.boxed(MappedPublic::into_test_if_public)
            }
            Event::ExternalAddressConfirmed(addr) => {
                panic!(
                    "State<MappedPublic>: Swarm confirmed external address {}, but {} was expected",
                    addr,
                    self.state.addr(),
                );
            }
            Event::AutonatClientTestOk(addr) | Event::AutonatClientTestFailed(addr) => {
                panic!(
                    "State<MappedPublic>: Autonat client reported address {}, but {} was expected",
                    addr,
                    self.state.addr(),
                );
            }
            _ => self,
        }
    }
}

impl OnEvent for State<Private> {
    fn on_event(self: Box<Self>, event: Event, _: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::LocalAddressChanged(addr) if self.state.addr() != &addr => {
                self.boxed(|state| state.into_test_if_public(addr))
            }
            Event::DefaultGatewayChanged(_) => {
                let local_addr = self.state.addr().clone();
                self.boxed(|state| state.into_test_if_public(local_addr))
            }
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use fixtures::*;
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use super::*;

    impl<S: PartialEq + 'static> DynPartialEq for State<S> {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn box_eq(&self, other: &dyn core::any::Any) -> bool {
            other.downcast_ref::<Self>() == Some(self)
        }
    }

    impl PartialEq for Box<dyn OnEvent> {
        fn eq(&self, other: &Self) -> bool {
            self.box_eq(other.as_any())
        }
    }

    impl PartialEq<&Self> for Box<dyn OnEvent> {
        fn eq(&self, other: &&Self) -> bool {
            self.box_eq(other.as_any())
        }
    }

    fn all_events<'a>() -> HashSet<TestEvent<'a>> {
        [
            autonat_failed(),
            autonat_failed_with_addr_mismatch(),
            autonat_ok(),
            autonat_ok_with_address_mismatch(),
            mapping_failed(),
            mapping_failed_with_addr_mismatch(),
            mapping_ok(),
            new_external_address_candidate(),
            external_address_confirmed(),
            external_address_confirmed_with_addr_mismatch(),
            default_gateway_changed(),
            local_address_changed(),
            other_from_swarm_event(),
        ]
        .into()
    }

    fn init_state_machine(
        init_state: Box<dyn OnEvent>,
        command_tx: UnboundedSender<Command>,
    ) -> StateMachine {
        StateMachine {
            inner: Some(init_state),
            command_tx: command_tx.into(),
        }
    }

    type TestCase<'a> = Vec<(
        // Source line number for debugging
        u32,
        // Factory of the initial state of the state machine
        Box<dyn Fn() -> Box<dyn OnEvent>>,
        // Expected transitions from the initial state
        Vec<(
            // Event that should cause the transition
            TestEvent<'a>,
            // Factory of the expected state after the transition
            Box<dyn Fn() -> Box<dyn OnEvent>>,
            // Expected command that should be emitted by the state machine
            // If `None`, no command is expected
            Option<Command>,
        )>,
    )>;

    #[expect(clippy::too_many_lines, reason = "Keep the test readable")]
    #[test]
    fn test_transitions_and_emitted_commands() {
        let expected_transitions: TestCase<'_> = vec![
            (
                line!(),
                Box::new(|| Uninitialized::for_test()),
                vec![(
                    new_external_address_candidate(),
                    Box::new(|| TestIfPublic::for_test(ADDR.clone())),
                    None,
                )],
            ),
            (
                line!(),
                Box::new(|| TestIfPublic::for_test(ADDR.clone())),
                vec![
                    (
                        autonat_failed(),
                        Box::new(|| TryMapAddress::for_test(ADDR.clone())),
                        Some(Command::MapAddress(ADDR.clone())),
                    ),
                    (
                        external_address_confirmed(),
                        Box::new(|| Public::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                ],
            ),
            (
                line!(),
                Box::new(|| TryMapAddress::for_test(ADDR.clone())),
                vec![
                    (
                        mapping_ok(),
                        Box::new(|| TestIfMappedPublic::for_test(ADDR.clone())),
                        Some(Command::NewExternalAddrCandidate(ADDR.clone())),
                    ),
                    (
                        mapping_failed(),
                        Box::new(|| Private::for_test(ADDR.clone())),
                        None,
                    ),
                ],
            ),
            (
                line!(),
                Box::new(|| TestIfMappedPublic::for_test(ADDR.clone())),
                vec![
                    (
                        external_address_confirmed(),
                        Box::new(|| MappedPublic::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_failed(),
                        Box::new(|| Private::for_test(ADDR.clone())),
                        None,
                    ),
                ],
            ),
            (
                line!(),
                Box::new(|| Public::for_test(ADDR.clone())),
                vec![
                    (
                        external_address_confirmed(),
                        Box::new(|| Public::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_ok(),
                        Box::new(|| Public::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_failed(),
                        Box::new(|| TestIfPublic::for_test(ADDR.clone())),
                        None,
                    ),
                ],
            ),
            (
                line!(),
                Box::new(|| MappedPublic::for_test(ADDR.clone())),
                vec![
                    (
                        external_address_confirmed(),
                        Box::new(|| MappedPublic::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_ok(),
                        Box::new(|| MappedPublic::for_test(ADDR.clone())),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_failed(),
                        Box::new(|| TestIfPublic::for_test(ADDR.clone())),
                        None,
                    ),
                ],
            ),
            (
                line!(),
                Box::new(|| Private::for_test(ADDR.clone())),
                vec![
                    (
                        local_address_changed(),
                        Box::new(|| TestIfPublic::for_test(ADDR_1.clone())),
                        None,
                    ),
                    (
                        default_gateway_changed(),
                        Box::new(|| TestIfPublic::for_test(ADDR.clone())),
                        None,
                    ),
                ],
            ),
        ];

        for (line, src_state, expected_transition) in expected_transitions {
            let mut expected_ignored_events: HashSet<TestEvent<'_>> = all_events();

            for (event, expected_state, expected_command) in expected_transition {
                expected_ignored_events.remove(&event);

                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state(), tx);

                let dbg_event = format!("{event:?}");
                sm.on_test_event(event);

                assert_eq!(
                    &expected_state(),
                    sm.inner.as_ref().unwrap(),
                    "Line {}, event `{}` caused transition `{:?} -> {:?}`, expected transition `{:?} -> {:?}`",
                    line,
                    dbg_event,
                    src_state(),
                    sm.inner,
                    src_state(),
                    expected_state(),
                );

                assert_eq!(
                    rx.try_recv(),
                    expected_command.ok_or(TryRecvError::Empty),
                    "Line {}, invalid command emitted for event `{}` in transition `{:?} -> {:?}`",
                    line,
                    dbg_event,
                    src_state(),
                    sm.inner.unwrap(),
                );
            }

            for event in expected_ignored_events {
                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state(), tx);

                let dbg_event = format!("{event:?}");
                sm.on_test_event(event);

                assert_eq!(
                    &src_state(),
                    sm.inner.as_ref().unwrap(),
                    "Event `{}` should be ignored but instead caused a transition `{:?} -> {:?}`",
                    dbg_event,
                    src_state(),
                    sm.inner.as_ref().unwrap()
                );

                assert!(
                    matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                    "Event `{}` should be ignored and not emit any command from `{:?}` state",
                    dbg_event,
                    src_state()
                );
            }
        }
    }

    mod fixtures {
        use std::{hash::Hash, str::FromStr as _, sync::LazyLock};

        use libp2p::{
            Multiaddr, PeerId, autonat,
            swarm::{
                FromSwarm, NewExternalAddrCandidate, NewExternalAddrOfPeer,
                behaviour::ExternalAddrConfirmed,
            },
        };

        use crate::behaviour::nat::{address_mapper, state_machine::StateMachine};

        #[derive(Debug)]
        pub enum TestEvent<'a> {
            AutonatClientFailed(&'static autonat::v2::client::Event),
            AutonatClientOk(autonat::v2::client::Event),
            AddressMapping(address_mapper::Event),
            FromSwarm(FromSwarm<'a>),
        }

        impl TestEvent<'_> {
            pub fn autonat_ok(tested_addr: Multiaddr) -> Self {
                TestEvent::AutonatClientOk(autonat::v2::client::Event {
                    tested_addr,
                    bytes_sent: 0,
                    server: *PEER_ID,
                    result: Ok(()),
                })
            }
        }

        impl<'a> StateMachine {
            pub(super) fn on_test_event(&mut self, event: TestEvent<'a>) {
                match event {
                    TestEvent::AutonatClientFailed(event) => {
                        self.on_event(event);
                    }
                    TestEvent::AutonatClientOk(event) => {
                        self.on_event(&event);
                    }
                    TestEvent::AddressMapping(event) => {
                        self.on_event(&event);
                    }
                    TestEvent::FromSwarm(event) => {
                        self.on_event(event);
                    }
                }
            }
        }

        impl Eq for TestEvent<'_> {
            fn assert_receiver_is_total_eq(&self) {}
        }

        impl PartialEq for TestEvent<'_> {
            fn eq(&self, other: &Self) -> bool {
                match (self, other) {
                    (Self::AutonatClientFailed(l), Self::AutonatClientFailed(r)) => {
                        l.tested_addr == r.tested_addr
                        && l.bytes_sent == r.bytes_sent
                        && l.server == r.server
                        // This is sufficient for testing purposes
                        && l.result.is_ok() == r.result.is_ok()
                    }
                    (Self::AutonatClientOk(l), Self::AutonatClientOk(r)) => {
                        l.tested_addr == r.tested_addr
                        && l.bytes_sent == r.bytes_sent
                        && l.server == r.server
                        // This is sufficient for testing purposes
                        && l.result.is_ok() == r.result.is_ok()
                    }
                    (Self::AddressMapping(l), Self::AddressMapping(r)) => l.eq(r),
                    (Self::FromSwarm(l), Self::FromSwarm(r)) => match (l, r) {
                        (
                            FromSwarm::NewExternalAddrCandidate(l),
                            FromSwarm::NewExternalAddrCandidate(r),
                        ) => l.addr == r.addr,
                        (
                            FromSwarm::ExternalAddrConfirmed(l),
                            FromSwarm::ExternalAddrConfirmed(r),
                        ) => l.addr == r.addr,
                        _ => false,
                    },
                    _ => core::mem::discriminant(self) == core::mem::discriminant(other),
                }
            }
        }

        impl Hash for TestEvent<'_> {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                match self {
                    Self::AutonatClientFailed(event) => {
                        event.tested_addr.hash(state);
                        event.bytes_sent.hash(state);
                        event.server.hash(state);
                        // This is sufficient for testing purposes
                        event.result.is_ok().hash(state);
                    }
                    Self::AutonatClientOk(event) => {
                        event.tested_addr.hash(state);
                        event.bytes_sent.hash(state);
                        event.server.hash(state);
                        // This is sufficient for testing purposes
                        event.result.is_ok().hash(state);
                    }
                    Self::AddressMapping(event) => event.hash(state),
                    Self::FromSwarm(event) => match event {
                        FromSwarm::NewExternalAddrCandidate(candidate) => {
                            candidate.addr.hash(state);
                        }
                        FromSwarm::ExternalAddrConfirmed(confirmed) => confirmed.addr.hash(state),
                        // Sufficient for testing purposes
                        _ => core::mem::discriminant(event).hash(state),
                    },
                }
            }
        }

        pub static ADDR: LazyLock<Multiaddr> =
            LazyLock::new(|| Multiaddr::from_str("/memory/0").unwrap());
        pub static ADDR_1: LazyLock<Multiaddr> =
            LazyLock::new(|| Multiaddr::from_str("/memory/1").unwrap());
        pub static AUTONAT_FAILED: LazyLock<BinaryCompatAutonatEvent> =
            LazyLock::new(|| BinaryCompatAutonatEvent::new("/memory/0"));
        pub static AUTONAT_FAILED_ADDR_1: LazyLock<BinaryCompatAutonatEvent> =
            LazyLock::new(|| BinaryCompatAutonatEvent::new("/memory/1"));
        pub static PEER_ID: LazyLock<PeerId> = LazyLock::new(PeerId::random);

        pub fn autonat_ok<'a>() -> TestEvent<'a> {
            TestEvent::autonat_ok(ADDR.clone())
        }

        pub fn autonat_ok_with_address_mismatch<'a>() -> TestEvent<'a> {
            TestEvent::autonat_ok(ADDR_1.clone())
        }

        #[derive(Debug)]
        pub struct BinaryCompatAutonatEvent {
            pub _tested_addr: Multiaddr,
            pub _bytes_sent: usize,
            pub _server: PeerId,
            pub _result: Result<(), BinaryCompatAutonatError>,
        }

        impl BinaryCompatAutonatEvent {
            pub fn new(tested_addr: &str) -> Self {
                Self {
                    _tested_addr: Multiaddr::from_str(tested_addr).unwrap(),
                    _bytes_sent: 0,
                    _server: *PEER_ID,
                    _result: Err(BinaryCompatAutonatError {
                        _inner: BinaryCompatDialBackError::NoConnection,
                    }),
                }
            }
        }

        #[derive(Debug)]
        pub struct BinaryCompatAutonatError {
            pub(crate) _inner: BinaryCompatDialBackError,
        }

        #[derive(thiserror::Error, Debug)]
        pub enum BinaryCompatDialBackError {
            #[error("server failed to establish a connection")]
            NoConnection,
        }

        pub fn autonat_failed<'a>() -> TestEvent<'a> {
            // SAFETY: layout and alignment of `BinaryCompatAutonatEvent` is compatible with
            // `autonat::v2::client::Event`
            TestEvent::AutonatClientFailed(unsafe {
                &*(&raw const *AUTONAT_FAILED).cast::<autonat::v2::client::Event>()
            })
        }

        pub fn autonat_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
            // SAFETY: layout and alignment of `BinaryCompatAutonatEvent` is compatible with
            // `autonat::v2::client::Event`
            TestEvent::AutonatClientFailed(unsafe {
                &*(&raw const *AUTONAT_FAILED_ADDR_1).cast::<autonat::v2::client::Event>()
            })
        }

        pub fn mapping_failed<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(ADDR.clone()))
        }

        pub fn mapping_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(ADDR_1.clone()))
        }

        pub fn mapping_ok<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::NewExternalMappedAddress(
                ADDR.clone(),
            ))
        }

        pub fn new_external_address_candidate<'a>() -> TestEvent<'a> {
            TestEvent::FromSwarm(FromSwarm::NewExternalAddrCandidate(
                NewExternalAddrCandidate { addr: &ADDR },
            ))
        }

        pub fn external_address_confirmed<'a>() -> TestEvent<'a> {
            TestEvent::FromSwarm(FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed {
                addr: &ADDR,
            }))
        }

        pub fn external_address_confirmed_with_addr_mismatch<'a>() -> TestEvent<'a> {
            TestEvent::FromSwarm(FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed {
                addr: &ADDR_1,
            }))
        }

        pub fn other_from_swarm_event<'a>() -> TestEvent<'a> {
            TestEvent::FromSwarm(FromSwarm::NewExternalAddrOfPeer(NewExternalAddrOfPeer {
                peer_id: *PEER_ID,
                addr: &ADDR_1,
            }))
        }

        pub fn default_gateway_changed<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::DefaultGatewayChanged)
        }

        pub fn local_address_changed<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::LocalAddressChanged(ADDR_1.clone()))
        }
    }
}
