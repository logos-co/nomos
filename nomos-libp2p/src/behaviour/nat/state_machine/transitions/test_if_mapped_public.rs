use crate::behaviour::nat::state_machine::{
    event::Event, states::TestIfMappedPublic, Command, CommandTx, OnEvent, State,
};

/// The `TestIfMappedPublic` state is responsible for testing if the mapped
/// address on the NAT-box is public. If the address is confirmed as public, it
/// transitions to the `MappedPublic` state. If the address is not public, it
/// transitions to the `Private` state.
///
/// ### Panics
///
/// This state will panic if it receives an event that does not match the
/// expected address to test.
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use super::Command;
    use crate::behaviour::nat::state_machine::{
        states::{MappedPublic, Private, TestIfMappedPublic},
        transitions::fixtures::{
            all_events, autonat_failed, autonat_failed_address_mismatch,
            external_address_confirmed, external_address_confirmed_address_mismatch, ADDR,
        },
        StateMachine,
    };

    #[test]
    fn external_address_confirmed_event_causes_transition_to_mapped_public() {
        let (tx, mut rx) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        state_machine.inner = Some(TestIfMappedPublic::for_test(ADDR.clone()));
        let event = external_address_confirmed();
        state_machine.on_test_event(event);
        assert_eq!(
            state_machine.inner.as_ref().unwrap(),
            &MappedPublic::for_test(ADDR.clone())
        );
        assert_eq!(
            rx.try_recv(),
            Ok(Command::ScheduleAutonatClientTest(ADDR.clone()))
        );
    }

    #[test]
    fn autonat_client_failed_causes_transition_to_private() {
        let (tx, mut rx) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        state_machine.inner = Some(TestIfMappedPublic::for_test(ADDR.clone()));
        let event = autonat_failed();
        state_machine.on_test_event(event);
        assert_eq!(
            state_machine.inner.as_ref().unwrap(),
            &Private::for_test(ADDR.clone())
        );
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[should_panic]
    #[test]
    fn address_mismatch_in_external_address_confirmed_event_causes_panic() {
        let (tx, _) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        state_machine.inner = Some(TestIfMappedPublic::for_test(ADDR.clone()));
        let event = external_address_confirmed_address_mismatch();
        state_machine.on_test_event(event);
    }

    #[should_panic]
    #[test]
    fn address_mismatch_in_autonat_failed_event_causes_panic() {
        let (tx, _) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        state_machine.inner = Some(TestIfMappedPublic::for_test(ADDR.clone()));
        let event = autonat_failed_address_mismatch();
        state_machine.on_test_event(event);
    }

    #[test]
    fn other_events_are_ignored() {
        let (tx, mut rx) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        state_machine.inner = Some(TestIfMappedPublic::for_test(ADDR.clone()));

        let mut other_events = all_events();
        other_events.remove(&external_address_confirmed());
        other_events.remove(&autonat_failed());

        for event in other_events {
            state_machine.on_test_event(event);
            assert_eq!(
                state_machine.inner.as_ref().unwrap(),
                &TestIfMappedPublic::for_test(ADDR.clone())
            );
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        }
    }
}
