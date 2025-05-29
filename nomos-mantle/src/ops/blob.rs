use serde::{Deserialize, Serialize};

use crate::ops::{opcode, serde_::WireOp};

pub type ChannelId = u64;
pub type BlobId = [u8; 32];
pub type TxHash = [u8; 32];

pub type Ed25519PublicKey = [u8; 32];

#[derive(Serialize, Deserialize, Clone)]
#[serde(
    from = "WireOp<{ opcode::BLOB }, BlobOp>",
    into = "WireOp<{ opcode::BLOB }, BlobOp>"
)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}

impl<'a> From<WireOp<{ opcode::BLOB }, BlobOp>> for BlobOp {
    fn from(value: WireOp<{ opcode::BLOB }, BlobOp>) -> Self {
        value.op
    }
}

impl Into<WireOp<{ opcode::BLOB }, BlobOp>> for BlobOp {
    fn into(self) -> WireOp<{ opcode::BLOB }, BlobOp> {
        WireOp { op: self }
    }
}
