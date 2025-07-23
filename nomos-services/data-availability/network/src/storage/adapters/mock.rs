use std::{collections::HashMap, sync::Mutex};

use libp2p::PeerId;
use nomos_core::block::BlockNumber;
use nomos_da_network_core::SubnetworkId;
use overwatch::{
    services::{relay::OutboundRelay, state::NoState, ServiceData},
    DynError,
};

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
}

#[derive(Default)]
pub struct MockStorage {
    state: Mutex<StorageState>,
}

#[async_trait::async_trait]
impl MembershipStorageAdapter<PeerId, SubnetworkId> for MockStorage {
    type StorageService = MockStorageService;

    fn new(_relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self {
        Self::default()
    }

    async fn store(
        &self,
        block_number: BlockNumber,
        assignations: Assignations<PeerId, SubnetworkId>,
    ) -> Result<(), DynError> {
        self.state
            .lock()
            .unwrap()
            .assignations
            .insert(block_number, assignations);
        Ok(())
    }

    async fn get(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Assignations<PeerId, SubnetworkId>>, DynError> {
        let state = self.state.lock().unwrap();
        Ok(state.assignations.get(&block_number).cloned())
    }

    async fn prune(&self) {
        todo!()
    }
}
