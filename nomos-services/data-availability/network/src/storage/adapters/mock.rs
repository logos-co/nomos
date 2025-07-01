use std::{collections::HashMap, sync::Mutex};

use libp2p::{Multiaddr, PeerId};
use nomos_core::block::BlockNumber;
use nomos_da_network_core::SubnetworkId;
use overwatch::services::{relay::OutboundRelay, state::NoState, ServiceData};

use crate::{membership::Assignations, MembershipStorageAdapter};

pub struct MockStorageService;

impl ServiceData for MockStorageService {
    type Settings = ();
    type State = NoState<()>;
    type StateOperator = ();
    type Message = ();
}

#[derive(Default)]
struct StorageState {
    assignations: HashMap<BlockNumber, Assignations<PeerId, SubnetworkId>>,
    addressbooks: HashMap<BlockNumber, HashMap<PeerId, Multiaddr>>,
}

#[derive(Default)]
pub struct MockStorage {
    state: Mutex<StorageState>,
}

impl MembershipStorageAdapter<PeerId, SubnetworkId> for MockStorage {
    type StorageService = MockStorageService;

    fn new(_relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self {
        Self::default()
    }

    fn store(
        &self,
        block_number: BlockNumber,
        assignations: Assignations<PeerId, SubnetworkId>,
        addressbook: HashMap<PeerId, Multiaddr>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.assignations.insert(block_number, assignations);
        state.addressbooks.insert(block_number, addressbook);
    }

    fn get(
        &self,
        block_number: BlockNumber,
    ) -> Option<(
        Assignations<PeerId, SubnetworkId>,
        HashMap<PeerId, Multiaddr>,
    )> {
        let (assignations, addressbook) = {
            let state = self.state.lock().unwrap();
            (
                state.assignations.get(&block_number).cloned(),
                state.addressbooks.get(&block_number).cloned(),
            )
        };

        assignations.zip(addressbook)
    }

    fn prune(&self) {
        todo!()
    }
}
