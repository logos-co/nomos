use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use nomos_core::header::HeaderId;
use thiserror::Error;

pub mod request_behaviour;

// Enhanced events that include block context
#[derive(Debug)]
pub enum HistoricSamplingEvent {
    SamplingSuccess {
        block_id: HeaderId,
        commitments: Vec<DaSharesCommitments>,
        shares: Vec<DaLightShare>,
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
