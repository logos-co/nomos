use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use nomos_core::{block::BlockNumber, da::BlobId, header::HeaderId};
use subnetworks_assignations::SubnetworkId;

use crate::protocols::sampling::errors::SamplingError;

pub mod request_behaviour;

// Enhanced events that include block context
#[derive(Debug)]
pub enum HistoricSamplingEvent {
    SamplingSuccess {
        block_number: BlockNumber,
        block_id: HeaderId,
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        light_share: Box<DaLightShare>,
    },
    CommitmentsSuccess {
        block_number: BlockNumber,
        block_id: HeaderId,
        blob_id: BlobId,
        commitments: Box<DaSharesCommitments>,
    },
    SamplingError {
        block_number: BlockNumber,
        block_id: HeaderId,
        error: SamplingError,
    },
}
