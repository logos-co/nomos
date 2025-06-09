use tokio::sync::mpsc::UnboundedSender;

mod events;
mod states;

use events::*;
use states::*;

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
pub(crate) struct StateMachine<Addr> {
    state: State<Addr>,
    command_tx: CommandTx<Addr>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum State<Addr> {
    Uninitialized(Uninitialized),
    TestIfPublic(TestIfPublic<Addr>),
    TryAddressMapping(TryAddressMapping<Addr>),
    TestIfMappedPublic(TestIfMappedPublic<Addr>),
    Public(Public<Addr>),
    MappedPublic(MappedPublic<Addr>),
    Private(Private<Addr>),
}

#[derive(Debug, Clone)]
struct CommandTx<Addr> {
    tx: UnboundedSender<Command<Addr>>,
}

/// Commands that can be issued by the state machine to `NatBehaviour`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum Command<Addr> {
    ScheduleAutonatClientTest(Addr),
    MapAddress(Addr),
    NewExternalAddrCandidate(Addr),
}

impl<Addr: Clone> StateMachine<Addr> {
    pub fn new(command_tx: UnboundedSender<Command<Addr>>) -> Self {
        Self {
            state: Uninitialized.into(),
            command_tx: command_tx.into(),
        }
    }

    pub fn on_event<Event>(&mut self, event: Event)
    where
        Event: TryInto<UninitializedEvent<Addr>, Error = ()>
            + TryInto<TestIfPublicEvent<Addr>, Error = ()>
            + TryInto<TryAddressMappingEvent<Addr>, Error = ()>
            + TryInto<TestIfMappedPublicEvent<Addr>, Error = ()>
            + TryInto<PublicEvent<Addr>, Error = ()>
            + TryInto<MappedPublicEvent<Addr>, Error = ()>
            + TryInto<PrivateEvent, Error = ()>,
    {
        let Ok(new_state) = self.state.clone().on_event(event, &self.command_tx) else {
            // If the event is not recognized by the current state, we ignore it.
            return;
        };

        self.state = new_state;
    }
}

impl<Addr: Clone> State<Addr> {
    pub fn on_event<Event>(self, event: Event, command_tx: &CommandTx<Addr>) -> Result<Self, ()>
    where
        Event: TryInto<UninitializedEvent<Addr>, Error = ()>
            + TryInto<TestIfPublicEvent<Addr>, Error = ()>
            + TryInto<TryAddressMappingEvent<Addr>, Error = ()>
            + TryInto<TestIfMappedPublicEvent<Addr>, Error = ()>
            + TryInto<PublicEvent<Addr>, Error = ()>
            + TryInto<MappedPublicEvent<Addr>, Error = ()>
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

trait OnEvent<Addr> {
    type Event;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr>;
}

impl<Addr: Clone> OnEvent<Addr> for Uninitialized {
    type Event = UninitializedEvent<Addr>;

    fn on_event(self, event: Self::Event, _: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::NewExternalAddressCandidate(addr) => TestIfPublic(addr).into(),
        }
    }
}

impl<Addr: Clone> OnEvent<Addr> for TestIfPublic<Addr> {
    type Event = TestIfPublicEvent<Addr>;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::ExternalAddressConfirmed(addr) => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                Public(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) => {
                command_tx.send(Command::MapAddress(addr.clone()));
                TryAddressMapping(addr).into()
            }
        }
    }
}

impl<Addr: Clone> OnEvent<Addr> for TryAddressMapping<Addr> {
    type Event = TryAddressMappingEvent<Addr>;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::_NewExternalMappedAddress(addr) => {
                command_tx.send(Command::NewExternalAddrCandidate(addr.clone()));
                TestIfMappedPublic(addr).into()
            }
            Self::Event::AddressMappingFailed(addr) => Private(addr).into(),
        }
    }
}

impl<Addr: Clone> OnEvent<Addr> for TestIfMappedPublic<Addr> {
    type Event = TestIfMappedPublicEvent<Addr>;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::ExternalAddressConfirmed(addr) => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                MappedPublic(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) => Private(addr).into(),
        }
    }
}

impl<Addr: Clone> OnEvent<Addr> for Public<Addr> {
    type Event = PublicEvent<Addr>;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr) => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                Public(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) => TestIfPublic(addr).into(),
        }
    }
}

impl<Addr: Clone> OnEvent<Addr> for MappedPublic<Addr> {
    type Event = MappedPublicEvent<Addr>;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::ExternalAddressConfirmed(addr)
            | Self::Event::AutonatClientTestOk(addr) => {
                command_tx.send(Command::ScheduleAutonatClientTest(addr.clone()));
                Public(addr).into()
            }
            Self::Event::AutonatClientTestFailed(addr) => Private(addr).into(),
        }
    }
}

impl<Addr> OnEvent<Addr> for Private<Addr> {
    type Event = PrivateEvent;

    fn on_event(self, event: Self::Event, _: &CommandTx<Addr>) -> State<Addr> {
        match event {
            Self::Event::_LocalAddressChanged | Self::Event::_DefaultGatewayChanged => {
                Uninitialized.into()
            }
        }
    }
}

impl<Addr> From<UnboundedSender<Command<Addr>>> for CommandTx<Addr> {
    fn from(tx: UnboundedSender<Command<Addr>>) -> Self {
        Self { tx }
    }
}

impl<Addr> CommandTx<Addr> {
    pub fn send(&self, command: Command<Addr>) {
        self.tx.send(command).expect("Channel not to be closed");
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fmt::Debug, hash::Hash, str::FromStr, sync::LazyLock};

    use libp2p::{
        autonat,
        swarm::{
            behaviour::ExternalAddrConfirmed, FromSwarm, NewExternalAddrCandidate,
            NewExternalAddrOfPeer,
        },
        Multiaddr, PeerId,
    };
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use super::*;
    use crate::behaviour::nat::address_mapper;

    #[derive(Debug)]
    enum TestEvent<'a> {
        AutonatClientFailed(&'static autonat::v2::client::Event),
        AutonatClientOk(autonat::v2::client::Event),
        AddressMapping(address_mapper::Event),
        /// For ExternalAddrConfirmed
        FromSwarm(FromSwarm<'a>),
        /// These events are not generated by any behaviour yet
        _DefaultGatewayChanged,
        _LocalAddressChanged,
    }

    impl<'a> StateMachine<Multiaddr> {
        fn on_test_event(&mut self, event: TestEvent<'a>) {
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
                TestEvent::_DefaultGatewayChanged => {
                    self.on_event(event);
                }
                TestEvent::_LocalAddressChanged => {
                    self.on_event(event);
                }
            }
        }
    }

    // TODO Remove this implementation once the two events are generated by a
    // behaviour
    impl_try_from_foreach!(TestEvent<'_>, event, [PrivateEvent], {
        match event {
            TestEvent::_DefaultGatewayChanged => Ok(Self::_DefaultGatewayChanged),
            TestEvent::_LocalAddressChanged => Ok(Self::_LocalAddressChanged),
            _ => unreachable!(),
        }
    });

    // TODO Remove this implementation once the two events are generated by a
    // behaviour
    impl_try_from_foreach!(
        TestEvent<'_>,
        event,
        [
            UninitializedEvent<Multiaddr>,
            TestIfPublicEvent<Multiaddr>,
            TryAddressMappingEvent<Multiaddr>,
            TestIfMappedPublicEvent<Multiaddr>,
            PublicEvent<Multiaddr>,
            MappedPublicEvent<Multiaddr>
        ],
        {
            match event {
                TestEvent::_DefaultGatewayChanged => Err(()),
                TestEvent::_LocalAddressChanged => Err(()),
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
                    (FromSwarm::ExternalAddrConfirmed(l), FromSwarm::ExternalAddrConfirmed(r)) => {
                        l.addr == r.addr
                    }
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
                    FromSwarm::NewExternalAddrCandidate(candidate) => candidate.addr.hash(state),
                    FromSwarm::ExternalAddrConfirmed(confirmed) => confirmed.addr.hash(state),
                    // Sufficient for testing purposes
                    _ => core::mem::discriminant(event).hash(state),
                },
                _ => core::mem::discriminant(self).hash(state),
            }
        }
    }

    static ADDR: LazyLock<Multiaddr> = LazyLock::new(|| Multiaddr::from_str("/memory/0").unwrap());
    static ADDR_MISMATCH: LazyLock<Multiaddr> =
        LazyLock::new(|| Multiaddr::from_str("/memory/1").unwrap());
    static AUTONAT_FAILED: LazyLock<BinaryCompatAutonatEvent> =
        LazyLock::new(|| BinaryCompatAutonatEvent::new("/memory/0"));
    static AUTONAT_FAILED_ADDR_MISMATCH: LazyLock<BinaryCompatAutonatEvent> =
        LazyLock::new(|| BinaryCompatAutonatEvent::new("/memory/0"));

    fn autonat_ok<'a>() -> TestEvent<'a> {
        TestEvent::AutonatClientOk(autonat::v2::client::Event {
            tested_addr: ADDR.clone(),
            bytes_sent: 0,
            server: PeerId::random(),
            result: Ok(()),
        })
    }

    fn autonat_ok_with_address_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::AutonatClientOk(autonat::v2::client::Event {
            tested_addr: ADDR_MISMATCH.clone(),
            bytes_sent: 0,
            server: PeerId::random(),
            result: Ok(()),
        })
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
                _server: PeerId::random(),
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

    fn autonat_failed<'a>() -> TestEvent<'a> {
        TestEvent::AutonatClientFailed(unsafe { std::mem::transmute(&*AUTONAT_FAILED) })
    }

    fn autonat_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::AutonatClientFailed(unsafe {
            std::mem::transmute(&*AUTONAT_FAILED_ADDR_MISMATCH)
        })
    }

    fn mapping_failed<'a>() -> TestEvent<'a> {
        TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(ADDR.clone()))
    }

    fn mapping_failed_with_addr_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::AddressMapping(address_mapper::Event::AddressMappingFailed(
            ADDR_MISMATCH.clone(),
        ))
    }

    fn mapping_ok<'a>() -> TestEvent<'a> {
        TestEvent::AddressMapping(address_mapper::Event::_NewExternalMappedAddress(
            ADDR.clone(),
        ))
    }

    fn mapping_ok_with_address_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::AddressMapping(address_mapper::Event::_NewExternalMappedAddress(
            ADDR_MISMATCH.clone(),
        ))
    }

    fn new_external_address_candidate<'a>() -> TestEvent<'a> {
        TestEvent::FromSwarm(FromSwarm::NewExternalAddrCandidate(
            NewExternalAddrCandidate { addr: &ADDR },
        ))
    }

    fn new_external_address_candidate_with_addr_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::FromSwarm(FromSwarm::NewExternalAddrCandidate(
            NewExternalAddrCandidate {
                addr: &ADDR_MISMATCH,
            },
        ))
    }

    fn external_address_confirmed<'a>() -> TestEvent<'a> {
        TestEvent::FromSwarm(FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed {
            addr: &ADDR,
        }))
    }

    fn external_address_confirmed_with_addr_mismatch<'a>() -> TestEvent<'a> {
        TestEvent::FromSwarm(FromSwarm::ExternalAddrConfirmed(ExternalAddrConfirmed {
            addr: &ADDR_MISMATCH,
        }))
    }

    fn other_from_swarm_event<'a>() -> TestEvent<'a> {
        TestEvent::FromSwarm(FromSwarm::NewExternalAddrOfPeer(NewExternalAddrOfPeer {
            peer_id: PeerId::random(),
            addr: &ADDR_MISMATCH,
        }))
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
            mapping_ok_with_address_mismatch(),
            new_external_address_candidate(),
            new_external_address_candidate_with_addr_mismatch(),
            external_address_confirmed(),
            external_address_confirmed_with_addr_mismatch(),
            TestEvent::_DefaultGatewayChanged,
            TestEvent::_LocalAddressChanged,
            other_from_swarm_event(),
        ]
        .into()
    }

    fn init_state_machine(
        init_state: State<Multiaddr>,
        tx: UnboundedSender<Command<Multiaddr>>,
    ) -> StateMachine<Multiaddr> {
        let mut sm = StateMachine::new(tx);
        sm.state = init_state;
        sm
    }

    #[test]
    fn test_transitions_and_emitted_commands() {
        let expected_transitions: Vec<(
            u32,
            State<Multiaddr>,
            Vec<(TestEvent<'_>, State<Multiaddr>, Option<Command<Multiaddr>>)>,
        )> = vec![
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
                        Public(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (
                        autonat_ok(),
                        Public(ADDR.clone()).into(),
                        Some(Command::ScheduleAutonatClientTest(ADDR.clone())),
                    ),
                    (autonat_failed(), Private(ADDR.clone()).into(), None),
                ],
            ),
            (
                line!(),
                Private(ADDR.clone()).into(),
                vec![
                    (TestEvent::_LocalAddressChanged, Uninitialized.into(), None),
                    (
                        TestEvent::_DefaultGatewayChanged,
                        Uninitialized.into(),
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
}
