use serde::{Deserialize, Serialize};

use crate::mantle::ops::{ChannelId, Ed25519PublicKey};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SetChannelKeysOp {
    channel: ChannelId,
    keys: Vec<Ed25519PublicKey>,
}
