use kzgrs_backend::common::ShareIndex;
use nomos_core::da::BlobId;
use serde::{Deserialize, Serialize};

use crate::common::LightShare;

#[repr(C)]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SampleErrorType {
    NotFound,
}

#[repr(C)]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SampleError {
    pub blob_id: BlobId,
    pub column_idx: ShareIndex,
    pub error_type: SampleErrorType,
    pub error_description: String,
}

impl SampleError {
    pub fn new(
        blob_id: BlobId,
        column_idx: ShareIndex,
        error_type: SampleErrorType,
        error_description: impl Into<String>,
    ) -> Self {
        Self {
            blob_id,
            column_idx,
            error_type,
            error_description: error_description.into(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SampleRequest {
    pub blob_id: BlobId,
    pub share_idx: ShareIndex,
}

impl SampleRequest {
    #[must_use]
    pub const fn new(blob_id: BlobId, share_idx: ShareIndex) -> Self {
        Self { blob_id, share_idx }
    }
}

#[repr(C)]
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SampleResponse {
    Share(LightShare),
    Error(SampleError),
}
