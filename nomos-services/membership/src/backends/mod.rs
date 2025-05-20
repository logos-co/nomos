use std::collections::HashMap;

use async_trait::async_trait;
use nomos_sdp::backends::SdpBackendError;
use nomos_sdp_core::{BlockNumber, FinalizedBlockEvent, ServiceType};
use overwatch::DynError;

use crate::MembershipProviders;

pub mod mock;

pub struct MembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, Settings>,
}

pub struct Settings {
    historical_block_delta: u64,
}

#[derive(Debug)]
pub enum MembershipBackendError {
    Other(DynError),
    Sdp(SdpBackendError),
    MockBackendError(DynError),
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
