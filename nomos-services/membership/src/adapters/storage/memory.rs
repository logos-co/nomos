use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

// Dummy service for testing
pub struct DummyStorageService;

impl ServiceData for DummyStorageService {
    type Settings = ();
    type State = ();
    type StateOperator = ();
    type Message = ();
}

#[derive(Clone)]
pub struct InMemoryStorageAdapter {
    data: Arc<Mutex<InMemoryStorage>>,
}

#[derive(Default)]
struct InMemoryStorage {
    active_sessions: HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
    forming_sessions: HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
    latest_block: Option<BlockNumber>,
}

impl InMemoryStorageAdapter {
    #[must_use]
    pub fn new_for_testing() -> Self {
        Self {
            data: Arc::new(Mutex::new(InMemoryStorage::default())),
        }
    }
}

#[async_trait]
impl super::MembershipStorageAdapter for InMemoryStorageAdapter {
    type StorageService = DummyStorageService;

    fn new(_relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self {
        Self::new_for_testing()
    }

    async fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError> {
        self.data
            .lock()
            .unwrap()
            .active_sessions
            .insert(service_type, (session_id, providers.clone()));
        Ok(())
    }

    async fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .active_sessions
            .get(&service_type)
            .cloned())
    }

    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), DynError> {
        self.data.lock().unwrap().latest_block = Some(block_number);
        Ok(())
    }

    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, DynError> {
        Ok(self.data.lock().unwrap().latest_block)
    }

    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError> {
        self.data
            .lock()
            .unwrap()
            .forming_sessions
            .insert(service_type, (session_id, providers.clone()));
        Ok(())
    }

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .forming_sessions
            .get(&service_type)
            .cloned())
    }
}
