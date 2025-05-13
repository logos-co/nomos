use std::collections::HashMap;

use async_trait::async_trait;
use nomos_sdp_core::{
    BlockNumber, DeclarationUpdate, FinalizedBlockEvent, ProviderInfo, ServiceType,
};
use overwatch::DynError;

pub mod mock;

pub struct MembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, SnapshotSettings>,
}

pub enum SnapshotSettings {
    Block(usize),
}

#[derive(Debug, Clone)]
pub struct MembershipEntry {
    pub block_number: BlockNumber,
    pub data: HashMap<ServiceType, HashMap<ProviderInfo, DeclarationUpdate>>,
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
    ) -> Result<HashMap<ProviderInfo, DeclarationUpdate>, DynError>;
    async fn update(&mut self, update: FinalizedBlockEvent) -> Result<(), DynError>;
}
