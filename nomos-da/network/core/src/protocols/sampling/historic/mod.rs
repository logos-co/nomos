use std::collections::HashMap;

use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use nomos_core::{da::BlobId, header::HeaderId};

use crate::protocols::sampling::errors::HistoricSamplingError;

pub mod request_behaviour;

#[derive(Debug)]
pub enum HistoricSamplingEvent {
    SamplingSuccess {
        block_id: HeaderId,
        commitments: HashMap<BlobId, DaSharesCommitments>,
        shares: HashMap<BlobId, Vec<DaLightShare>>,
    },
    SamplingError {
        block_id: HeaderId,
        error: HistoricSamplingError,
    },
}
