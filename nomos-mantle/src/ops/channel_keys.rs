use serde::{Deserialize, Serialize};

use crate::{
    gas::{Gas, GasConstants, GasPrice},
    ops::{ChannelId, Ed25519PublicKey},
};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SetChannelKeysOp {
    channel: ChannelId,
    keys: Vec<Ed25519PublicKey>,
}

impl GasPrice for SetChannelKeysOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        Constants::SET_CHANNEL_KEYS_BASE_GAS
            + Constants::SET_CHANNEL_KEY_BYTE_GAS
                * size_of::<Ed25519PublicKey>() as u64
                * self.keys.len() as u64
    }
}
