use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::{ChannelId, Ed25519PublicKey, MsgId};
use crate::{
    crypto::{Digest as _, Hasher},
    utils::ed25519_serde::Ed25519Hex,
};

#[serde_as]
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct InscriptionOp {
    pub channel_id: ChannelId,
    /// Message to be written in the blockchain
    pub inscription: Vec<u8>,
    /// Enforce that this inscription comes after this tx
    pub parent: MsgId,
    #[serde_as(as = "Ed25519Hex")]
    pub signer: Ed25519PublicKey,
}

impl InscriptionOp {
    #[must_use]
    pub fn id(&self) -> MsgId {
        let mut hasher = Hasher::new();
        hasher.update(self.payload_bytes());
        MsgId(hasher.finalize().into())
    }

    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.channel_id.as_ref());
        buff.extend_from_slice(&self.inscription);
        buff.extend_from_slice(self.parent.as_ref());
        buff.extend_from_slice(self.signer.as_ref());
        buff.freeze()
    }
}
