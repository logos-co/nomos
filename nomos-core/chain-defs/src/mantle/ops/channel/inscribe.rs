use serde::{Deserialize, Serialize};

use super::{ChannelId, Ed25519PublicKey, MsgId};
use crate::crypto::{Digest as _, Hasher};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct InscriptionOp {
    channel_id: ChannelId,
    /// Message to be written in the blockchain
    inscription: Vec<u8>,
    /// Enforce that this inscription comes after this tx
    parent: MsgId,
    signer: Ed25519PublicKey,
}

impl InscriptionOp {
    #[must_use]
    pub const fn new(
        channel_id: ChannelId,
        inscription: Vec<u8>,
        parent: MsgId,
        signer: Ed25519PublicKey,
    ) -> Self {
        Self {
            channel_id,
            inscription,
            parent,
            signer,
        }
    }

    #[must_use]
    pub const fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }

    #[must_use]
    pub fn inscription(&self) -> &[u8] {
        &self.inscription
    }

    #[must_use]
    pub const fn parent(&self) -> &MsgId {
        &self.parent
    }

    #[must_use]
    pub const fn signer(&self) -> &Ed25519PublicKey {
        &self.signer
    }

    #[must_use]
    pub fn id(&self) -> MsgId {
        let mut hasher = Hasher::new();
        hasher.update(self.channel_id.as_ref());
        hasher.update(&self.inscription);
        hasher.update(self.parent.as_ref());
        hasher.update(self.signer.as_ref());
        MsgId(hasher.finalize().into())
    }
}
