use std::collections::HashMap;

use async_trait::async_trait;
use nomos_core::sdp::{FinalizedBlockEvent, ServiceType};
use overwatch::DynError;
use thiserror::Error;

use crate::MembershipProviders;

pub mod mock;

#[derive(Debug, Error)]
pub enum MembershipBackendError {
    #[error("Other error: {0}")]
    Other(#[from] DynError),

    #[error("The block received is not greater than the last known block")]
    BlockFromPast,
}

pub type NewSesssion = Option<HashMap<ServiceType, MembershipProviders>>;

#[async_trait]
pub trait MembershipBackend {
    type Settings: Send + Sync;

    fn init(settings: Self::Settings) -> Self;

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError>;

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<NewSesssion, MembershipBackendError>;
}
