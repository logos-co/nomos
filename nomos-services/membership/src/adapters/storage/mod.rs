pub mod memory;

use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};

#[async_trait::async_trait]
pub trait MembershipStorageAdapter: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    // Session snapshots per service type
    async fn save_session_snapshot(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error>;

    async fn load_session_snapshot(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
    ) -> Result<Option<HashMap<ProviderId, BTreeSet<Locator>>>, Self::Error>;

    // Latest processed block
    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), Self::Error>;

    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, Self::Error>;

    // Forming session state per service type
    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error>;

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, Self::Error>;
}
