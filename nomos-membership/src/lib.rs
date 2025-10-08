use std::collections::{BTreeSet, HashMap};

use nomos_core::sdp::{Locator, ProviderId, ServiceType};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::session::Session;

pub mod membership;
pub mod session;
pub mod storage;

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
