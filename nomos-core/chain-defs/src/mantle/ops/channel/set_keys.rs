use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::{ChannelId, Ed25519PublicKey};
use crate::utils::ed25519_serde::Ed25519Hex;

#[serde_as]
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SetKeysOp {
    pub channel: ChannelId,
    #[serde_as(as = "Vec<Ed25519Hex>")]
    pub keys: Vec<Ed25519PublicKey>,
}

impl SetKeysOp {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.channel.as_ref());
        for key in &self.keys {
            buff.extend_from_slice(key.as_ref());
        }
        buff.freeze()
    }
}
