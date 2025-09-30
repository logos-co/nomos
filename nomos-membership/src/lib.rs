use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use nomos_core::{
    block::SessionNumber,
    sdp::{FinalizedBlockEvent, Locator, ProviderId, ServiceType},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod membership;
mod storage;

#[derive(Debug, Clone)]
pub struct Session {
    session_number: SessionNumber,
    providers: HashMap<ProviderId, BTreeSet<Locator>>,
}

pub type DynError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub type NewSesssion = Option<HashMap<ServiceType, Session>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipConfig {
    pub session_sizes: HashMap<ServiceType, u32>,
    pub session_zero_providers: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,
}

#[derive(Debug, Error)]
pub enum MembershipError {
    #[error("Other error: {0}")]
    Other(#[from] DynError),

    #[error("The block received is not greater than the last known block")]
    BlockFromPast,

    #[error("Not found")]
    NotFound,
}

#[async_trait]
pub trait Membership {
    type Storage;

    fn new(settings: MembershipConfig, storage_adapter: Self::Storage) -> Self;

    fn get_latest_providers(&self, service_type: ServiceType) -> Result<Session, MembershipError>;

    async fn update(&mut self, update: FinalizedBlockEvent)
    -> Result<NewSesssion, MembershipError>;
}
