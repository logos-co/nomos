use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};

pub(crate) const DA_COLUMNS: u64 = 1024;
pub(crate) const DA_ELEMENT_SIZE: u64 = 32;

use super::{ChannelId, Ed25519PublicKey, MsgId};
use crate::{crypto::Digest as _, da::BlobId, mantle::gas::Gas};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BlobOp {
    pub channel: ChannelId,
    pub blob: BlobId,
    pub blob_size: u64,
    pub da_storage_gas_price: Gas,
    pub parent: MsgId,
    pub signer: Ed25519PublicKey,
}

impl BlobOp {
    #[must_use]
    pub fn id(&self) -> MsgId {
        let mut hasher = crate::crypto::Hasher::new();
        hasher.update(self.payload_bytes());
        MsgId(hasher.finalize().into())
    }

    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.channel.as_ref());
        buff.extend_from_slice(self.blob.as_ref());
        buff.extend_from_slice(self.blob_size.to_le_bytes().as_ref());
        buff.extend_from_slice(self.da_storage_gas_price.to_le_bytes().as_ref());
        buff.extend_from_slice(self.parent.as_ref());
        buff.extend_from_slice(self.signer.as_ref());
        buff.freeze()
    }
}
