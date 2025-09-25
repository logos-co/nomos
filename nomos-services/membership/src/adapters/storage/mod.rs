pub mod rocksdb;

use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};

#[async_trait::async_trait]
pub trait MembershipStorageAdapter: Send + Sync {
    type StorageService: ServiceData;

    fn new(relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self;

    async fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError>;

    async fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError>;

    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), DynError>;
    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, DynError>;

    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError>;

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError>;
}
