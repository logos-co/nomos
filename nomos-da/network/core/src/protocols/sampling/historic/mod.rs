use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use nomos_core::header::HeaderId;

use crate::protocols::sampling::errors::SamplingError;

pub mod request_behaviour;

// Enhanced events that include block context
#[derive(Debug)]
pub enum HistoricSamplingEvent {
    SamplingSuccess {
        block_id: HeaderId,
        commitments: Box<DaSharesCommitments>,
        shares: Vec<DaLightShare>,
    },
    SamplingError {
        block_id: HeaderId,
        error: SamplingError,
    },
}
