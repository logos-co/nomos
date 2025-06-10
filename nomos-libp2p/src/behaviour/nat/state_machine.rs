use libp2p::Multiaddr;
use tokio::sync::mpsc::UnboundedSender;

mod events;
mod states;

use events::*;
use states::*;

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
pub(crate) struct StateMachine {
    state: State,
    command_tx: CommandTx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Uninitialized(Uninitialized),
    TestIfPublic(TestIfPublic),
    TryAddressMapping(TryAddressMapping),
    TestIfMappedPublic(TestIfMappedPublic),
    Public(Public),
    MappedPublic(MappedPublic),
    Private(Private),
}

#[derive(Debug, Clone)]
struct CommandTx {
    tx: UnboundedSender<Command>,
}

/// Commands that can be issued by the state machine to `NatBehaviour`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    ScheduleAutonatClientTest(Multiaddr),
    MapAddress(Multiaddr),
    NewExternalAddrCandidate(Multiaddr),
}

impl StateMachine {
    pub fn new(command_tx: UnboundedSender<Command>) -> Self {
        Self {
            state: Uninitialized.into(),
            command_tx: command_tx.into(),
        }
    }

    pub fn on_event<Event>(&mut self, event: Event)
    where
        Event: TryInto<UninitializedEvent, Error = ()>
            + TryInto<TestIfPublicEvent, Error = ()>
            + TryInto<TryAddressMappingEvent, Error = ()>
            + TryInto<TestIfMappedPublicEvent, Error = ()>
            + TryInto<PublicEvent, Error = ()>
            + TryInto<MappedPublicEvent, Error = ()>
            + TryInto<PrivateEvent, Error = ()>,
    {
        let Ok(new_state) = self.state.clone().on_event(event, &self.command_tx) else {
            // If the event is not recognized by the current state, we ignore it.
            return;
        };

        self.state = new_state;
    }
}

impl State {
    pub fn on_event<Event>(self, event: Event, command_tx: &CommandTx) -> Result<Self, ()>
    where
        Event: TryInto<UninitializedEvent, Error = ()>
            + TryInto<TestIfPublicEvent, Error = ()>
            + TryInto<TryAddressMappingEvent, Error = ()>
            + TryInto<TestIfMappedPublicEvent, Error = ()>
            + TryInto<PublicEvent, Error = ()>
            + TryInto<MappedPublicEvent, Error = ()>
            + TryInto<PrivateEvent, Error = ()>,
    {
        Ok(match self {
            State::Uninitialized(x) => x.on_event(event.try_into()?, command_tx),
            State::TestIfPublic(x) => x.on_event(event.try_into()?, command_tx),
            State::TryAddressMapping(x) => x.on_event(event.try_into()?, command_tx),
            State::TestIfMappedPublic(x) => x.on_event(event.try_into()?, command_tx),
            State::Public(x) => x.on_event(event.try_into()?, command_tx),
            State::MappedPublic(x) => x.on_event(event.try_into()?, command_tx),
            State::Private(x) => x.on_event(event.try_into()?, command_tx),
        })
    }
}

trait OnEvent {
    type Event;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State;
}

impl OnEvent for Uninitialized {
    type Event = UninitializedEvent;

    fn on_event(self, event: Self::Event, _: &CommandTx) -> State {
        match event {
            Self::Event::NewExternalAddressCandidate(addr) => TestIfPublic(addr).into(),
        }
    }
}

impl OnEvent for TestIfPublic {
    type Event = TestIfPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed(addr) if self.0 == addr => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                Public(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) if self.0 == addr => {
                command_tx.send(Command::MapAddress(addr.clone()));
                TryAddressMapping(addr).into()
            }
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestFailed(addr) => {
                tracing::error!(
                    "Autonat client reported address {}, but {} was expected",
                    addr,
                    self.0,
                );

                self.into()
            }
        }
    }
}

impl OnEvent for TryAddressMapping {
    type Event = TryAddressMappingEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::_NewExternalMappedAddress(addr) => {
                command_tx.send(Command::NewExternalAddrCandidate(addr.clone()));
                TestIfMappedPublic(addr).into()
            }
            Self::Event::AddressMappingFailed(addr) if self.0 == addr => Private(addr).into(),
            Self::Event::AddressMappingFailed(addr) => {
                tracing::error!(
                    "Address mapper reported failure for address {}, but {} was expected",
                    addr,
                    self.0,
                );

                self.into()
            }
        }
    }
}

impl OnEvent for TestIfMappedPublic {
    type Event = TestIfMappedPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed(addr) if self.0 == addr => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                MappedPublic(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) if self.0 == addr => Private(addr).into(),
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestFailed(addr) => {
                tracing::error!(
                    "Autonat client reported address {}, but {} was expected",
                    addr,
                    self.0,
                );

                self.into()
            }
        }
    }
}

impl OnEvent for Public {
    type Event = PublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr)
                if self.0 == addr =>
            {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                Public(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) if self.0 == addr => {
                TestIfPublic(addr).into()
            }
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr)
            | Self::Event::AutonatClientTestFailed(addr) => {
                tracing::error!(
                    "Autonat client reported address {}, but {} was expected",
                    addr,
                    self.0,
                );

                self.into()
            }
        }
    }
}

impl OnEvent for MappedPublic {
    type Event = MappedPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr)
                if self.0 == addr =>
            {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                MappedPublic(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) if self.0 == addr => {
                TestIfPublic(addr).into()
            }
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr)
            | Self::Event::AutonatClientTestFailed(addr) => {
                tracing::error!(
                    "Autonat client reported address {}, but {} was expected",
                    addr,
                    self.0,
                );

                self.into()
            }
        }
    }
}

impl OnEvent for Private {
    type Event = PrivateEvent;

    fn on_event(self, event: Self::Event, _: &CommandTx) -> State {
        match event {
            Self::Event::_LocalAddressChanged(addr) => TestIfPublic(addr).into(),
            Self::Event::_DefaultGatewayChanged => TestIfPublic(self.0).into(),
        }
    }
}

impl From<UnboundedSender<Command>> for CommandTx {
    fn from(tx: UnboundedSender<Command>) -> Self {
        Self { tx }
    }
}

impl CommandTx {
    pub fn send(&self, command: Command) {
        self.tx.send(command).expect("Channel not to be closed");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use fixtures::*;
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use super::*;

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
            TestEvent::DefaultGatewayChanged,
            local_address_changed(),
            other_from_swarm_event(),
        ]
        .into()
    }

    fn init_state_machine(init_state: State, tx: UnboundedSender<Command>) -> StateMachine {
        let mut sm = StateMachine::new(tx);
        sm.state = init_state;
        sm
    }

    #[test]
    fn test_transitions_and_emitted_commands() {
        let expected_transitions: Vec<(u32, State, Vec<(TestEvent<'_>, State, Option<Command>)>)> = vec![
            (
                line!(),
                Uninitialized.into(),
                vec![(
                    new_external_address_candidate(),
                    TestIfPublic(ADDR.clone()).into(),
                    None,
                )],
            ),
            (
                line!(),
                TestIfPublic(ADDR.clone()).into(),
                vec![
                    (
                        autonat_failed(),
                        TryAddressMapping(ADDR.clone()).into(),
                        Some(Command::MapAddress(ADDR.clone())),
                    ),
                    (
                        external_address_confirmed(),
                        Public(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                ],
            ),
            (
                line!(),
                TryAddressMapping(ADDR.clone()).into(),
                vec![
                    (
                        mapping_ok(),
                        TestIfMappedPublic(ADDR.clone()).into(),
                        Some(Command::NewExternalAddrCandidate(ADDR.clone())),
                    ),
                    (mapping_failed(), Private(ADDR.clone()).into(), None),
                ],
            ),
            (
                line!(),
                TestIfMappedPublic(ADDR.clone()).into(),
                vec![
                    (
                        external_address_confirmed(),
                        MappedPublic(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (autonat_failed(), Private(ADDR.clone()).into(), None),
                ],
            ),
            (
                line!(),
                Public(ADDR.clone()).into(),
                vec![
                    (
                        external_address_confirmed(),
                        Public(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_ok(),
                        Public(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (autonat_failed(), TestIfPublic(ADDR.clone()).into(), None),
                ],
            ),
            (
                line!(),
                MappedPublic(ADDR.clone()).into(),
                vec![
                    (
                        external_address_confirmed(),
                        MappedPublic(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_ok(),
                        MappedPublic(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (autonat_failed(), TestIfPublic(ADDR.clone()).into(), None),
                ],
            ),
            (
                line!(),
                Private(ADDR.clone()).into(),
                vec![
                    (
                        local_address_changed(),
                        TestIfPublic(ADDR_1.clone()).into(),
                        None,
                    ),
                    (
                        TestEvent::DefaultGatewayChanged,
                        TestIfPublic(ADDR.clone()).into(),
                        None,
                    ),
                ],
            ),
        ];

        for (line, src_state, expected_transition) in expected_transitions {
            let mut expected_ignored_events: HashSet<TestEvent<'_>> = all_events();

            eprintln!(
                "A Line {}, len {} state {:#?} ignored events: {:#?}",
                line,
                expected_ignored_events.len(),
                src_state,
                expected_ignored_events
            );

            for (event, expected_state, expected_command) in expected_transition {
                eprintln!("event: {:#?}", event);

                expected_ignored_events.remove(&event);

                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state.clone(), tx);

                let dbg_event = format!("{:?}", event);
                sm.on_test_event(event);

                assert_eq!(
                    expected_state, sm.state,
                    "Line {}, event `{}` caused transition `{:?} -> {:?}`, expected transition `{:?} -> {:?}`",
                    line, dbg_event, src_state, sm.state, src_state, expected_state,
                );
                assert_eq!(
                    rx.try_recv(),
                    expected_command.ok_or(TryRecvError::Empty),
                    "Line {}, invalid command emitted for event `{}` in transition `{:?} -> {:?}`",
                    line,
                    dbg_event,
                    src_state,
                    sm.state,
                );
            }

            eprintln!(
                "B Line {}, len {} state {:#?} ignored events: {:#?}",
                line,
                expected_ignored_events.len(),
                src_state,
                expected_ignored_events
            );

            for event in expected_ignored_events {
                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state.clone(), tx);

                let dbg_event = format!("{:?}", event);
                sm.on_test_event(event);

                assert_eq!(
                    src_state, sm.state,
                    "Event `{}` should be ignored but instead caused a transition `{:?} -> {:?}`",
                    dbg_event, src_state, sm.state
                );
                assert!(
                    matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                    "Event `{}` should be ignored and not emit any command from `{:?}` state",
                    dbg_event,
                    src_state
                );
            }
        }
    }

    mod fixtures {
        use std::{hash::Hash, str::FromStr, sync::LazyLock};

        use libp2p::{
            autonat,
            swarm::{
                behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate,
                NewExternalAddrOfPeer,
            },
            Multiaddr, PeerId,
        };

        use crate::behaviour::nat::{
            address_mapper,
            state_machine::{events::*, impl_try_from, impl_try_from_foreach, StateMachine},
        };

        #[derive(Debug)]
        pub enum TestEvent<'a> {
            AutonatClientFailed(&'static autonat::v2::client::Event),
            AutonatClientOk(autonat::v2::client::Event),
            AddressMapping(address_mapper::Event),
            /// For ExternalAddrConfirmed
            FromSwarm(FromSwarm<'a>),
            /// These events are not generated by any behaviour yet
            DefaultGatewayChanged,
            LocalAddressChanged(Multiaddr),
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
                    TestEvent::DefaultGatewayChanged => {
                        self.on_event(event);
                    }
                    TestEvent::LocalAddressChanged(_) => {
                        self.on_event(event);
                    }
                }
            }
        }

        // TODO Remove this implementation once the two events are generated by a
        // behaviour
        impl_try_from_foreach!(TestEvent<'_>, event, [PrivateEvent], {
            match event {
                TestEvent::DefaultGatewayChanged => Ok(Self::_DefaultGatewayChanged),
                TestEvent::LocalAddressChanged(addr) => Ok(Self::_LocalAddressChanged(addr)),
                _ => unreachable!(),
            }
        });

        // TODO Remove this implementation once the two events are generated by a
        // behaviour
        impl_try_from_foreach!(
            TestEvent<'_>,
            event,
            [
                UninitializedEvent,
                TestIfPublicEvent,
                TryAddressMappingEvent,
                TestIfMappedPublicEvent,
                PublicEvent,
                MappedPublicEvent
            ],
            {
                match event {
                    TestEvent::DefaultGatewayChanged => Err(()),
                    TestEvent::LocalAddressChanged(_) => Err(()),
                    _ => unreachable!(),
                }
            }
        );

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
                            candidate.addr.hash(state)
                        }
                        FromSwarm::ExternalAddrConfirmed(confirmed) => confirmed.addr.hash(state),
                        // Sufficient for testing purposes
                        _ => core::mem::discriminant(event).hash(state),
                    },
                    _ => core::mem::discriminant(self).hash(state),
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
            TestEvent::AutonatClientFailed(unsafe { std::mem::transmute(&*AUTONAT_FAILED) })
        }

        pub fn autonat_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
            TestEvent::AutonatClientFailed(unsafe { std::mem::transmute(&*AUTONAT_FAILED_ADDR_1) })
        }

        pub fn mapping_failed<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(ADDR.clone()))
        }

        pub fn mapping_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(ADDR_1.clone()))
        }

        pub fn mapping_ok<'a>() -> TestEvent<'a> {
            TestEvent::AddressMapping(address_mapper::Event::_NewExternalMappedAddress(
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

        pub fn local_address_changed<'a>() -> TestEvent<'a> {
            TestEvent::LocalAddressChanged(ADDR_1.clone())
        }
    }
}
