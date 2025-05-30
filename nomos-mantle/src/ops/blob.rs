use serde::{Deserialize, Serialize};

use crate::ops::TxHash;

pub type ChannelId = u64;
pub type BlobId = [u8; 32];

pub type Ed25519PublicKey = [u8; 32];

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}
