use crate::behaviour::nat::state_machine::{
    event::Event,
    states::{
        MappedPublic, Private, Public, TestIfMappedPublic, TestIfPublic, TryMapAddress,
        Uninitialized,
    },
    Command, CommandTx, OnEvent, State,
};

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
