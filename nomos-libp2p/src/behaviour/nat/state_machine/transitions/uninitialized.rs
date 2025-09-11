use crate::behaviour::nat::state_machine::{
    event::Event, states::Uninitialized, CommandTx, OnEvent, State,
};

/// The `Uninitialized` state is the starting point of the NAT state machine. In
/// this state, the state machine is waiting for an external address candidate
/// to be provided. Once it receives a candidate, it transitions to the
/// `TestIfPublic` state to verify if the address is public or not.
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    use crate::behaviour::nat::state_machine::{
        states::{TestIfPublic, Uninitialized},
        transitions::fixtures::{all_events, new_external_address_candidate, ADDR},
        StateMachine,
    };

    #[test]
    fn new_external_address_candidate_event_causes_transition() {
        let (tx, mut rx) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        let event = new_external_address_candidate();
        state_machine.on_test_event(event);
        assert_eq!(
            state_machine.inner.as_ref().unwrap(),
            &TestIfPublic::for_test(ADDR.clone())
        );
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn other_events_are_ignored() {
        let (tx, mut rx) = unbounded_channel();
        let mut state_machine = StateMachine::new(tx);
        let mut other_events = all_events();
        other_events.remove(&new_external_address_candidate());
        for event in other_events {
            state_machine.on_test_event(event);
            assert_eq!(
                state_machine.inner.as_ref().unwrap(),
                &Uninitialized::for_test()
            );
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        }
    }
}
