use std::{collections::HashMap, sync::Mutex};

use libp2p::PeerId;
use nomos_core::block::SessionNumber;
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
    assignations: HashMap<SessionNumber, Assignations<PeerId, SubnetworkId>>,
    addressbook: HashMap<PeerId, libp2p::Multiaddr>,
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
        session_id: SessionNumber,
        assignations: Assignations<PeerId, SubnetworkId>,
    ) -> Result<(), DynError> {
        self.state
            .lock()
            .unwrap()
            .assignations
            .insert(session_id, assignations);
        Ok(())
    }

    async fn get(
        &self,
        session_id: SessionNumber,
    ) -> Result<Option<Assignations<PeerId, SubnetworkId>>, DynError> {
        let state = self.state.lock().unwrap();
        Ok(state.assignations.get(&session_id).cloned())
    }

    async fn store_addresses(
        &self,
        ids: HashMap<PeerId, libp2p::Multiaddr>,
    ) -> Result<(), DynError> {
        self.state.lock().unwrap().addressbook.extend(ids);
        Ok(())
    }

    async fn get_address(&self, id: PeerId) -> Result<Option<libp2p::Multiaddr>, DynError> {
        let state = self.state.lock().unwrap();
        Ok(state.addressbook.get(&id).cloned())
    }

    async fn prune(&self) {
        todo!()
    }
}
