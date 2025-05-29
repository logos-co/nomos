use nomos_core::wire;
use serde::{Deserialize, Serialize};

use crate::{
    ops::OpCode,
    txs::TxError,
    wire::{MantleTx, Op},
};

pub type ChannelId = u64;
pub type BlobId = [u8; 32];
pub type TxHash = [u8; 32];

pub type Ed25519PublicKey = [u8; 32];

#[derive(Serialize, Deserialize)]
pub struct BlobTx {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}

impl TryFrom<MantleTx> for BlobTx {
    type Error = TxError;

    fn try_from(tx: MantleTx) -> Result<Self, Self::Error> {
        if let [
            Op {
                opcode: OpCode::Blob,
                payload,
            },
        ] = tx.ops.as_slice()
        {
            Ok(wire::deserialize(payload).map_err(|_| Self::Error::UnableToDecode)?)
        } else {
            Err(Self::Error::UnableToDecode)
        }
    }
}
