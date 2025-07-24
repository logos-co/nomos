use serde::{Deserialize, Serialize};

use super::{ChannelId, Ed25519PublicKey};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SetKeysOp {
    channel: ChannelId,
    keys: Vec<Ed25519PublicKey>,
}

impl SetKeysOp {
    #[must_use]
    pub const fn new(channel: ChannelId, keys: Vec<Ed25519PublicKey>) -> Self {
        Self { channel, keys }
    }

    #[must_use]
    pub const fn channel(&self) -> &ChannelId {
        &self.channel
    }

    #[must_use]
    pub fn keys(&self) -> &[Ed25519PublicKey] {
        &self.keys
    }
}
