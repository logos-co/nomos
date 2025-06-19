use serde::{Deserialize, Serialize};

use crate::mantle::{
    gas::{Gas, GasConstants, GasPrice},
    ops::{ChannelId, Ed25519PublicKey},
    tx::TxHash,
};

pub type BlobId = [u8; 32];

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}

impl GasPrice for BlobOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        let ordered = u64::from(self.after_tx.is_some());
        Constants::BLOB_BASE_GAS
            + Constants::BLOB_BYTE_GAS * self.blob_size
            + Constants::BLOB_ORDERING_GAS * ordered
    }
}
