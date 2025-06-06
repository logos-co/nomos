use tokio::sync::mpsc::UnboundedSender;

mod events;
mod states;

use events::*;
use states::*;

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
pub struct StateMachine<Addr> {
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
pub enum Command<Addr> {
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

    pub fn on_event(&mut self, event: impl TryInto<Event<Addr>>) {
        self.state = self.state.take().on_event(event, &self.command_tx);
    }
}

impl<Addr: Clone> State<Addr> {
    /// Event conversion is 2-staged:
    /// 1. Convert from libp2p to a generic Event type.
    /// 2. Convert from generic Event type to the state specific event type in
    ///    `on_event_inner`.
    /// This allows us to handle events in each state in a type-safe manner,
    /// while still allowing compy-paste boilerplate when converting from libp2p
    /// events.
    pub fn on_event(self, event: impl TryInto<Event<Addr>>, command_tx: &CommandTx<Addr>) -> Self {
        let Ok(event) = event.try_into() else {
            return self;
        };

        let current_state = self.clone();

        let Ok(new_state) = self.on_event_inner(event, command_tx) else {
            return current_state;
        };

        new_state
    }

    pub fn take(&mut self) -> State<Addr> {
        std::mem::replace(self, Uninitialized.into())
    }

    fn on_event_inner(
        self,
        event: Event<Addr>,
        command_tx: &CommandTx<Addr>,
    ) -> Result<State<Addr>, ()> {
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
            Self::Event::NewExternalMappedAddress(addr) => {
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
            Self::Event::LocalAddressChanged | Self::Event::DefaultGatewayChanged => {
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
    use std::collections::BTreeSet;

    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use super::*;

    fn all_events() -> BTreeSet<Event> {
        [
            Event::AutonatClientTestFailed,
            Event::AutonatClientTestOk,
            Event::AddressMappingFailed,
            Event::_DefaultGatewayChanged,
            Event::ExternalAddressConfirmed,
            Event::_LocalAddressChanged,
            Event::_NewExternalMappedAddress,
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
        let expected_transitions: Vec<(State, Vec<(Event, State, Option<Command>)>)> = vec![
            (
                Uninitialized.into(),
                vec![(
                    Event::NewExternalAddressCandidate,
                    TestIfPublic.into(),
                    None,
                )],
            ),
            (
                TestIfPublic.into(),
                vec![
                    (
                        Event::AutonatClientTestFailed,
                        TryAddressMapping.into(),
                        Some(Command::MapAddress),
                    ),
                    (
                        Event::ExternalAddressConfirmed,
                        Public.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                ],
            ),
            (
                TryAddressMapping.into(),
                vec![
                    (
                        Event::_NewExternalMappedAddress,
                        TestIfMappedPublic.into(),
                        Some(Command::NewExternalAddrCandidate),
                    ),
                    (Event::AddressMappingFailed, Private.into(), None),
                ],
            ),
            (
                TestIfMappedPublic.into(),
                vec![
                    (
                        Event::ExternalAddressConfirmed,
                        MappedPublic.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                    (Event::AutonatClientTestFailed, Private.into(), None),
                ],
            ),
            (
                Public.into(),
                vec![
                    (
                        Event::ExternalAddressConfirmed,
                        Public.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                    (
                        Event::AutonatClientTestOk,
                        Public.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                    (Event::AutonatClientTestFailed, TestIfPublic.into(), None),
                ],
            ),
            (
                MappedPublic.into(),
                vec![
                    (
                        Event::ExternalAddressConfirmed,
                        Public.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                    (
                        Event::AutonatClientTestOk,
                        Public.into(),
                        Some(Command::ScheduleAutonatClientTest),
                    ),
                    (Event::AutonatClientTestFailed, Private.into(), None),
                ],
            ),
            (
                Private.into(),
                vec![
                    (Event::_LocalAddressChanged, TestIfPublic.into(), None),
                    (Event::_DefaultGatewayChanged, TestIfPublic.into(), None),
                ],
            ),
        ];

        for (src_state, expected_transition) in expected_transitions {
            let mut expected_ignored_events: BTreeSet<Event> = all_events();

            for (event, expected_state, expected_command) in expected_transition {
                expected_ignored_events.remove(&event);

                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state, tx);

                sm.on_event(event);

                assert_eq!(
                    expected_state, sm.state,
                    "Event `{:?}` caused transition `{:?} -> {:?}`, expected transition `{:?} -> {:?}`",
                    event, src_state, sm.state, src_state, expected_state,
                );
                assert_eq!(
                    rx.try_recv(),
                    expected_command.ok_or(TryRecvError::Empty),
                    "Invalid command emitted for event `{:?}` in transition `{:?} -> {:?}`",
                    event,
                    src_state,
                    sm.state,
                );
            }

            for event in expected_ignored_events {
                let (tx, mut rx) = unbounded_channel();
                let mut sm = init_state_machine(src_state, tx);

                sm.on_event(event);

                assert_eq!(
                    src_state, sm.state,
                    "Event `{:?}` should be ignored but instead caused a transition `{:?} -> {:?}`",
                    event, src_state, sm.state
                );
                assert!(
                    matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                    "Event `{:?}` should be ignored and not emit any command from `{:?}` state",
                    event,
                    src_state
                );
            }
        }
    }
}
