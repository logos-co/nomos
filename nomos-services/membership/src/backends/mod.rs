use std::collections::HashMap;

use async_trait::async_trait;
use nomos_sdp::backends::SdpBackendError;
use nomos_sdp_core::{FinalizedBlockEvent, ServiceType};
use overwatch::DynError;

use crate::MembershipSnapshot;

pub mod mock;

pub struct MembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, SnapshotSettings>,
}

pub enum SnapshotSettings {
    Block(usize),
}

#[derive(Debug)]
pub enum MembershipBackendError {
    Other(DynError),
    Sdp(SdpBackendError),
}

#[async_trait]
pub trait MembershipBackend {
    type Settings: Send + Sync;

    fn init(settings: Self::Settings) -> Self;
    // Get the snapshot at the given index for the specified service type.
    // The snapshot index a sequential order of the service type specific time
    // snapshots. index = 0 is the latest snapshot
    // index > 0 is the index of the snapshot in the past
    // index < 0 is the index of the snapshot relative to the latest snapshot
    async fn get_snapshot_at(
        &self,
        service_type: ServiceType,
        index: i32,
    ) -> Result<MembershipSnapshot, MembershipBackendError>;
    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<HashMap<ServiceType, MembershipSnapshot>, MembershipBackendError>;
}
