use std::collections::HashMap;

use async_trait::async_trait;
use nomos_sdp::backends::FinalizedBlockEvent;
use nomos_sdp_core::ServiceType;
use overwatch::DynError;

pub mod deque;

pub struct MembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, SnapshotSettings>,
}

pub enum SnapshotSettings {
    Block(usize),
}

#[async_trait]
pub trait MembershipBackend {
    // todo: introduce error type instead of DynError
    fn init(settings: MembershipBackendSettings) -> Self;

    // Get the snapshot at the given index for the specified service type.
    // The snapshot index a sequential order of the service type specific time
    // snapshots. index = 0 is the latest snapshot
    // index > 0 is the index of the snapshot in the past
    // index < 0 is the index of the snapshot relative to the latest snapshot
    async fn get_snapshot_at(&self, service_type: ServiceType, index: i32) -> Result<(), DynError>;
    async fn update(&self, update: FinalizedBlockEvent) -> Result<(), DynError>;
}
