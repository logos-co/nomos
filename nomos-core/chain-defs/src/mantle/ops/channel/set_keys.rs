use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};

use super::{ChannelId, Ed25519PublicKey};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SetKeysOp {
    pub channel: ChannelId,
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
