use std::collections::HashSet;

use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use nomos_core::header::HeaderId;
use thiserror::Error;

pub mod request_behaviour;

#[derive(Debug)]
pub enum HistoricSamplingEvent {
    SamplingSuccess {
        block_id: HeaderId,
        commitments: HashSet<DaSharesCommitments>,
        shares: HashSet<DaLightShare>,
    },
    SamplingError {
        block_id: HeaderId,
        error: HistoricSamplingError,
    },
}

#[derive(Error, Debug)]
pub enum HistoricSamplingError {
    #[error("Historic sampling failed")]
    SamplingFailed,
    #[error("Internal server error: {0}")]
    InternalServerError(String),
}
