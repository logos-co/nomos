use tokio::sync::mpsc::UnboundedSender;

mod events;
mod states;

use events::*;
use states::*;

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
pub struct StateMachine {
    state: State,
    command_tx: CommandTx,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum State {
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
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Command {
    ScheduleAutonatClientTest,
    MapAddress,
    NewExternalAddrCandidate,
}

impl StateMachine {
    pub fn new(command_tx: UnboundedSender<Command>) -> Self {
        Self {
            state: TestIfPublic.into(),
            command_tx: command_tx.into(),
        }
    }

    pub fn on_event(&mut self, event: impl TryInto<Event>) {
        self.state = self.state.on_event(event, &self.command_tx);
    }
}

impl State {
    /// Event conversion is 2-staged
    /// 1. Convert from libp2p to a generic Event type
    /// 2. Convert from generic Event type to the state specific event type
    /// This allows us to handle events in each state in a type-safe manner,
    /// while still allowing compy-paste boilerplate when converting from libp2p
    /// events.
    pub fn on_event(self, event: impl TryInto<Event>, command_tx: &CommandTx) -> Self {
        let Ok(event) = event.try_into() else {
            return self;
        };

        let Ok(new_state) = self.on_event_inner(event, command_tx) else {
            return self;
        };

        new_state
    }

    fn on_event_inner(self, event: Event, command_tx: &CommandTx) -> Result<State, ()> {
        Ok(match self {
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

impl OnEvent for TestIfPublic {
    type Event = TestIfPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed => {
                command_tx.send(Command::ScheduleAutonatClientTest);
                Public.into()
            }
            Self::Event::AutonatClientTestFailed => {
                command_tx.send(Command::MapAddress);
                TryAddressMapping.into()
            }
        }
    }
}

impl OnEvent for TryAddressMapping {
    type Event = TryAddressMappingEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::NewExternalMappedAddress => {
                command_tx.send(Command::NewExternalAddrCandidate);
                TestIfMappedPublic.into()
            }
            Self::Event::AddressMappingFailed => Private.into(),
        }
    }
}

impl OnEvent for TestIfMappedPublic {
    type Event = TestIfMappedPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed => {
                command_tx.send(Command::ScheduleAutonatClientTest);
                MappedPublic.into()
            }
            Self::Event::AutonatClientTestFailed => Private.into(),
        }
    }
}

impl OnEvent for Public {
    type Event = PublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed | Self::Event::AutonatClientTestOk => {
                command_tx.send(Command::ScheduleAutonatClientTest);
                Public.into()
            }
            Self::Event::AutonatClientTestFailed => TestIfPublic.into(),
        }
    }
}

impl OnEvent for MappedPublic {
    type Event = MappedPublicEvent;

    fn on_event(self, event: Self::Event, command_tx: &CommandTx) -> State {
        match event {
            Self::Event::ExternalAddressConfirmed | Self::Event::AutonatClientTestOk => {
                command_tx.send(Command::ScheduleAutonatClientTest);
                Public.into()
            }
            Self::Event::AutonatClientTestFailed => Private.into(),
        }
    }
}

impl OnEvent for Private {
    type Event = PrivateEvent;

    fn on_event(self, event: Self::Event, _: &CommandTx) -> State {
        match event {
            Self::Event::LocalAddressChanged | Self::Event::DefaultGatewayChanged => {
                TestIfPublic.into()
            }
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
