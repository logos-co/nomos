use std::sync::Arc;

use ed25519::signature::Verifier as _;
use nomos_core::mantle::{
    ops::channel::{set_keys::SetKeysOp, ChannelId, Ed25519PublicKey as PublicKey, MsgId},
    TxHash,
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Error;

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channels {
    pub channels: rpds::HashTrieMapSync<ChannelId, ChannelState>,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelState {
    pub tip: MsgId,
    // avoid cloning the keys every new message
    pub keys: Arc<[PublicKey]>,
}

impl Default for Channels {
    fn default() -> Self {
        Self::new()
    }
}

impl Channels {
    pub fn apply_msg(
        mut self,
        channel_id: ChannelId,
        parent: &MsgId,
        msg: MsgId,
        signer: &PublicKey,
    ) -> Result<Self, Error> {
        let channel = self
            .channels
            .get(&channel_id)
            .cloned()
            .unwrap_or_else(|| ChannelState {
                tip: MsgId::root(),
                keys: vec![*signer].into(),
            });

        if *parent != channel.tip {
            return Err(Error::InvalidParent {
                channel_id,
                parent: (*parent).into(),
                actual: channel.tip.into(),
            });
        }

        if !channel.keys.contains(signer) {
            return Err(Error::UnauthorizedSigner {
                channel_id,
                signer: format!("{signer:?}"),
            });
        }

        self.channels = self.channels.insert(
            channel_id,
            ChannelState {
                tip: msg,
                keys: Arc::clone(&channel.keys),
            },
        );
        Ok(self)
    }

    pub fn set_keys(
        mut self,
        channel_id: ChannelId,
        op: &SetKeysOp,
        sig: &ed25519::Signature,
        tx_hash: &TxHash,
    ) -> Result<Self, Error> {
        if op.keys.is_empty() {
            return Err(Error::EmptyKeys { channel_id });
        }

        if let Some(channel) = self.channels.get_mut(&channel_id) {
            if channel.keys[0].verify(tx_hash.as_ref(), sig).is_err() {
                return Err(Error::InvalidSignature);
            }
            channel.keys = op.keys.clone().into();
        } else {
            self.channels = self.channels.insert(
                channel_id,
                ChannelState {
                    tip: MsgId::root(),
                    keys: op.keys.clone().into(),
                },
            );
        }

        Ok(self)
    }

    #[must_use]
    pub fn new() -> Self {
        Self {
            channels: rpds::HashTrieMapSync::new_sync(),
        }
    }
}
