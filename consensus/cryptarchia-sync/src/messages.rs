use bytes::Bytes;
use cryptarchia_engine::Slot;
use nomos_core::header::HeaderId;
use serde::{Deserialize, Serialize};

/// Blocks are serialized using nomos-core's wire format.
pub type SerialisedBlock = Bytes;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum GetTipResponse {
    /// A success response.
    Tip {
        tip: HeaderId,
        slot: Slot,
        height: u64,
    },
    /// A response indicating that the request failed.
    Failure(String),
}
