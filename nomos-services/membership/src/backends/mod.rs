use std::collections::HashMap;

use async_trait::async_trait;
use nomos_core::sdp::{FinalizedBlockEvent, ServiceType};
use overwatch::DynError;
use thiserror::Error;

use crate::{adapters::storage::MembershipStorageAdapter, MembershipProviders};

pub mod membership;

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
    type StorageAdapter: MembershipStorageAdapter;

    fn init(settings: Self::Settings, storage_adapter: Self::StorageAdapter) -> Self;

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError>;

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<NewSesssion, MembershipBackendError>;
}
