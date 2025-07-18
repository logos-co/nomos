use std::collections::HashMap;

use async_trait::async_trait;
use nomos_core::{
    block::BlockNumber,
    sdp::{FinalizedBlockEvent, ServiceType},
};
use overwatch::DynError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::MembershipProviders;

pub mod mock;

pub struct MembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, MembershipBackendServiceSettings>,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct MembershipBackendServiceSettings {
    pub historical_block_delta: u64,
}

#[derive(Debug, Error)]
pub enum MembershipBackendError {
    #[error("Other error: {0}")]
    Other(#[from] DynError),

    #[error("The block received is not greater than the last known block")]
    BlockFromPast,
}

#[async_trait]
pub trait MembershipBackend {
    type Settings: Send + Sync;

    fn init(settings: Self::Settings) -> Self;
    async fn get_providers_at(
        &self,
        service_type: ServiceType,
        block_number: BlockNumber,
    ) -> Result<MembershipProviders, MembershipBackendError>;

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError>;

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<HashMap<ServiceType, MembershipProviders>, MembershipBackendError>;
}
