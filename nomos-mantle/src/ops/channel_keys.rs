use serde::{Deserialize, Serialize};

use crate::ops::{ChannelId, Ed25519PublicKey};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetChannelKeysOp {
    channel: ChannelId,
    keys: Vec<Ed25519PublicKey>,
}
