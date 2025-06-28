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
pub struct MockStorage {
    assignations: Mutex<HashMap<BlockNumber, Assignations<PeerId, SubnetworkId>>>,
    addressbooks: Mutex<HashMap<BlockNumber, HashMap<PeerId, Multiaddr>>>,
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
        self.assignations
            .lock()
            .unwrap()
            .insert(block_number, assignations);

        let mut addressbooks = self.addressbooks.lock().unwrap();
        addressbooks.insert(block_number, addressbook);
    }

    fn get(
        &self,
        block_number: BlockNumber,
    ) -> Option<(
        Assignations<PeerId, SubnetworkId>,
        HashMap<PeerId, Multiaddr>,
    )> {
        let assignations = self
            .assignations
            .lock()
            .unwrap()
            .get(&block_number)
            .cloned();

        let addressbook = self
            .addressbooks
            .lock()
            .unwrap()
            .get(&block_number)
            .cloned();

        match (assignations, addressbook) {
            (Some(assignations), Some(addressbook)) => Some((assignations, addressbook)),
            _ => None,
        }
    }
}
