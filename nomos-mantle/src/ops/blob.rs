use serde::{Deserialize, Serialize};

use crate::ops::{ChannelId, Ed25519PublicKey, TxHash};

pub type BlobId = [u8; 32];

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}
