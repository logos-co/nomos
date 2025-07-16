use serde::{Deserialize, Serialize};

pub(crate) const DA_COLUMNS: u64 = 1024;
pub(crate) const DA_ELEMENT_SIZE: u64 = 32;

use crate::mantle::{
    gas::Gas,
    ops::{ChannelId, Ed25519PublicKey},
    tx::TxHash,
};

pub type BlobId = [u8; 32];

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub da_storage_gas_price: Gas,
    pub after_tx: Option<TxHash>,
    pub signer: Ed25519PublicKey,
}
