use crate::behaviour::nat::state_machine::{
    event::Event,
    states::{
        MappedPublic, Private, Public, TestIfMappedPublic, TestIfPublic, TryMapAddress,
        Uninitialized,
    },
    Command, CommandTx, OnEvent, State,
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

/// The `TestIfPublic` state is responsible for testing if the provided address
/// is public. If the address is confirmed as public, it transitions to the
/// `Public` state. If the address is not public, it transitions to the
/// `TryMapAddress` state to attempt mapping the address to some public-facing
/// address on the NAT-box.
///
/// ### Panics
///
/// This state will panic if it receives an event that does not match the
/// expected address to test.
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

/// The `TryMapAddress` state is responsible for attempting to map the address
/// to a public-facing address on the NAT-box. If the mapping is successful, it
/// transitions to the `TestIfMappedPublic` state to verify if the mapped
/// address is indeed public. If the mapping fails, it transitions to the
/// `Private` state.
///
/// ### Panics
///
/// This state will panic if it receives a mapping failure event that does not
/// match the expected address to map.
impl OnEvent for State<TryMapAddress> {
    fn on_event(self: Box<Self>, event: Event, command_tx: &CommandTx) -> Box<dyn OnEvent> {
        match event {
            Event::NewExternalMappedAddress(external_addr_of_nat_box) => {
                command_tx.force_send(Command::NewExternalAddrCandidate(
                    external_addr_of_nat_box.clone(),
                ));
                self.boxed(|state| state.into_test_if_mapped_public(external_addr_of_nat_box))
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

/// The `Public` state represents a state where the node's address is known and
/// confirmed to be publicly reachable. In this state, the address is
/// periodically tested by the `autonat` client to ensure it remains valid. If
/// the address is found to be unreachable, the state machine transitions to the
/// `TestIfPublic` state to re-evaluate the address.
///
/// ### Panics
///
/// This state will panic if it receives an event that does not match the
/// expected address to test.
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

/// The `MappedPublic` state represents a state where the node's address is
/// known and confirmed to be mapped to a publicly reachable address on the
/// NAT-box. In this state, the address is periodically tested by the `autonat`
/// client to ensure it remains valid. If the address is found to be
/// unreachable, the state machine transitions to the `TestIfMappedPublic` state
/// to re-evaluate the address.
///
/// ### Panics
///
/// This state will panic if it receives an event that does not match the
/// expected address to test.
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

/// The `Private` state represents a state where the node's address is known,
/// but it is not publicly reachable, and it has not been successfully mapped to
/// a publicly reachable address on the NAT-box. In this state, the state
/// machine waits for a change in the local address or the default gateway to
/// re-evaluate the address in the `TestIfPublic` state.
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
