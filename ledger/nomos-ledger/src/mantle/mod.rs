pub mod channel;

use ed25519::signature::Verifier as _;
use nomos_core::mantle::{
    ops::{channel::ChannelId, Op, OpProof},
    AuthenticatedMantleTx, GasConstants,
};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Invalid parent {parent:?} for channel {channel_id:?}, expected {actual:?}")]
    InvalidParent {
        channel_id: ChannelId,
        parent: [u8; 32],
        actual: [u8; 32],
    },
    #[error("Unauthorized signer {signer:?} for channel {channel_id:?}")]
    UnauthorizedSigner {
        channel_id: ChannelId,
        signer: String,
    },
    #[error("Invalid keys for channel {channel_id:?}")]
    EmptyKeys { channel_id: ChannelId },
    #[error("Unsupported operation")]
    UnsupportedOp,
    #[error("Invalid signature")]
    InvalidSignature,
}

/// Tracks mantle ops
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct LedgerState {
    channels: channel::Channels,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            channels: channel::Channels::new(),
        }
    }

    pub fn try_apply_tx<Id, Constants: GasConstants>(
        mut self,
        tx: impl AuthenticatedMantleTx,
    ) -> Result<Self, Error> {
        let tx_hash = tx.hash();
        for (op, proof) in tx.ops_with_proof() {
            match (op, proof) {
                (Op::ChannelBlob(op), Some(OpProof::Ed25519Sig(sig))) => {
                    // these proofs could be verified even before reaching this point
                    // as you only need the op itself to validate the signature
                    op.signer()
                        .verify(tx_hash.as_ref(), sig)
                        .map_err(|_| Error::InvalidSignature)?;
                    self.channels = self.channels.apply_msg(
                        *op.channel_id(),
                        op.parent(),
                        op.id(),
                        op.signer(),
                    )?;
                }
                (Op::ChannelInscribe(op), Some(OpProof::Ed25519Sig(sig))) => {
                    op.signer()
                        .verify(tx_hash.as_ref(), sig)
                        .map_err(|_| Error::InvalidSignature)?;
                    self.channels = self.channels.apply_msg(
                        *op.channel_id(),
                        op.parent(),
                        op.id(),
                        op.signer(),
                    )?;
                }
                (Op::ChannelSetKeys(op), Some(OpProof::Ed25519Sig(sig))) => {
                    self.channels = self.channels.set_keys(*op.channel(), op, sig, &tx_hash)?;
                }
                _ => {
                    return Err(Error::UnsupportedOp);
                }
            }
        }

        Ok(self)
    }
}
