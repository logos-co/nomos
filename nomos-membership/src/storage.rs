use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};

use crate::DynError;

pub type SessionMembers = (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>);

pub trait MembershipStorage {
    fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError>;

    fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<SessionMembers>, DynError>;

    fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), DynError>;

    fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, DynError>;

    fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError>;

    fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<SessionMembers>, DynError>;
}
