use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};

use crate::DynError;

#[async_trait::async_trait]
pub trait MembershipStorage {
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
