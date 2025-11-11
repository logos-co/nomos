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
