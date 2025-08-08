use nomos_core::{da::BlobId, mantle::SignedMantleTx};
use serde::{Deserialize, Serialize};

use crate::{
    common::{Share, ShareRequest},
    SubnetworkId,
};

#[repr(C)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DispersalErrorType {
    ChunkSize,
    BlobVerification,
    TxVerification,
}

#[repr(C)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DispersalError {
    pub blob_id: BlobId,
    pub error_type: DispersalErrorType,
    pub error_description: String,
}

impl DispersalError {
    pub fn new(
        blob_id: BlobId,
        error_type: DispersalErrorType,
        error_description: impl Into<String>,
    ) -> Self {
        Self {
            blob_id,
            error_type,
            error_description: error_description.into(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DispersalRequest {
    Share(ShareRequest),
    Tx(SignedMantleTx),
}

impl DispersalRequest {
    #[must_use]
    pub const fn new_share(share: Share, subnetwork_id: SubnetworkId) -> Self {
        Self::Share(ShareRequest {
            share,
            subnetwork_id,
        })
    }

    #[must_use]
    pub const fn new_tx(tx: SignedMantleTx) -> Self {
        Self::Tx(tx)
    }
}

#[repr(C)]
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DispersalResponse {
    Tx(BlobId),
    BlobId(BlobId),
    Error(DispersalError),
}
