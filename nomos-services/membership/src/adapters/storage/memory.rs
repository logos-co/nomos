use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};

use crate::adapters::storage::MembershipStorageAdapter;

#[derive(Debug, Clone)]
pub struct InMemoryStorageAdapter {
    active_sessions: HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
    forming_sessions: HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
    latest_block: Option<BlockNumber>,
}

impl InMemoryStorageAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active_sessions: HashMap::new(),
            forming_sessions: HashMap::new(),
            latest_block: None,
        }
    }
}

#[async_trait::async_trait]
impl MembershipStorageAdapter for InMemoryStorageAdapter {
    type Error = std::convert::Infallible;

    async fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error> {
        self.active_sessions
            .insert(service_type, (session_id, providers.clone()));
        Ok(())
    }

    async fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, Self::Error> {
        Ok(self.active_sessions.get(&service_type).cloned())
    }

    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), Self::Error> {
        self.latest_block = Some(block_number);
        Ok(())
    }

    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, Self::Error> {
        Ok(self.latest_block)
    }

    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error> {
        self.forming_sessions
            .insert(service_type, (session_id, providers.clone()));
        Ok(())
    }

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, Self::Error> {
        Ok(self.forming_sessions.get(&service_type).cloned())
    }
}

impl Default for InMemoryStorageAdapter {
    fn default() -> Self {
        Self::new()
    }
}
