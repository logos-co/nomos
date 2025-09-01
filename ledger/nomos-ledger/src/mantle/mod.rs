pub mod channel;
pub mod sdp;

use ed25519::signature::Verifier as _;
use nomos_core::{
    mantle::{
        ops::{channel::ChannelId, Op, OpProof},
        AuthenticatedMantleTx, GasConstants,
    },
    sdp::state::DeclarationStateError,
};
use sdp::SdpLedgerError;

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
    #[error("Sdp ledger error: {0:?}")]
    Sdp(#[from] SdpLedgerError),
}

impl From<DeclarationStateError> for Error {
    fn from(err: DeclarationStateError) -> Self {
        Self::Sdp(SdpLedgerError::SdpStateError(err))
    }
}

/// Tracks mantle ops
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq, Debug)]
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

    pub fn try_apply_tx<Constants: GasConstants>(
        mut self,
        tx: impl AuthenticatedMantleTx,
    ) -> Result<Self, Error> {
        let tx_hash = tx.hash();
        for (op, proof) in tx.ops_with_proof() {
            match (op, proof) {
                (Op::ChannelBlob(op), Some(OpProof::Ed25519Sig(sig))) => {
                    // these proofs could be verified even before reaching this point
                    // as you only need the op itself to validate the signature
                    op.signer
                        .verify(tx_hash.as_signing_bytes().as_ref(), sig)
                        .map_err(|_| Error::InvalidSignature)?;
                    self.channels =
                        self.channels
                            .apply_msg(op.channel, &op.parent, op.id(), &op.signer)?;
                }
                (Op::ChannelInscribe(op), Some(OpProof::Ed25519Sig(sig))) => {
                    op.signer
                        .verify(tx_hash.as_signing_bytes().as_ref(), sig)
                        .map_err(|_| Error::InvalidSignature)?;
                    self.channels =
                        self.channels
                            .apply_msg(op.channel_id, &op.parent, op.id(), &op.signer)?;
                }
                (Op::ChannelSetKeys(op), Some(OpProof::Ed25519Sig(sig))) => {
                    self.channels = self.channels.set_keys(op.channel, op, sig, &tx_hash)?;
                }
                _ => {
                    return Err(Error::UnsupportedOp);
                }
            }
        }

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
    use nomos_core::{
        mantle::{
            gas::MainnetGasConstants,
            ledger::Tx as LedgerTx,
            ops::channel::{blob::BlobOp, inscribe::InscriptionOp, set_keys::SetKeysOp, MsgId},
            MantleTx, SignedMantleTx, Transaction as _,
        },
        proofs::zksig::DummyZkSignature,
    };

    use super::*;

    fn create_test_keys() -> (SigningKey, VerifyingKey) {
        create_test_keys_with_seed(0)
    }

    fn create_test_keys_with_seed(seed: u8) -> (SigningKey, VerifyingKey) {
        let signing_key = SigningKey::from_bytes(&[seed; 32]);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    fn create_test_tx_with_ops(ops: Vec<Op>) -> SignedMantleTx {
        let ledger_tx = LedgerTx::new(vec![], vec![]);
        let mantle_tx = MantleTx {
            ops,
            ledger_tx,
            execution_gas_price: 1,
            storage_gas_price: 1,
        };

        SignedMantleTx {
            ops_profs: vec![None; mantle_tx.ops.len()],
            ledger_tx_proof: DummyZkSignature::prove(
                nomos_core::proofs::zksig::ZkSignaturePublic {
                    pks: vec![],
                    msg_hash: mantle_tx.hash().into(),
                },
            ),
            mantle_tx,
        }
    }

    fn create_signed_tx(op: Op, signing_key: &SigningKey) -> SignedMantleTx {
        create_multi_signed_tx(vec![op], vec![signing_key])
    }

    fn create_multi_signed_tx(ops: Vec<Op>, signing_keys: Vec<&SigningKey>) -> SignedMantleTx {
        let mut tx = create_test_tx_with_ops(ops);
        let tx_hash = tx.hash();
        tx.ops_profs = signing_keys
            .into_iter()
            .map(|key| {
                Some(OpProof::Ed25519Sig(
                    key.sign(tx_hash.as_signing_bytes().as_ref()),
                ))
            })
            .collect();
        tx
    }

    #[test]
    fn test_channel_blob_operation() {
        let ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([1; 32]);

        let blob_op = BlobOp {
            channel: channel_id,
            blob: [42; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let tx = create_signed_tx(Op::ChannelBlob(blob_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_channel_inscribe_operation() {
        let ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([2; 32]);

        let inscribe_op = InscriptionOp {
            channel_id,
            inscription: vec![1, 2, 3, 4],
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let tx = create_signed_tx(Op::ChannelInscribe(inscribe_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert!(result.is_ok());

        let new_state = result.unwrap();
        assert!(new_state.channels.channels.contains_key(&channel_id));
    }

    #[test]
    fn test_channel_set_keys_operation() {
        let ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([3; 32]);

        let set_keys_op = SetKeysOp {
            channel: channel_id,
            keys: vec![verifying_key],
        };

        let tx = create_signed_tx(Op::ChannelSetKeys(set_keys_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert!(result.is_ok());

        let new_state = result.unwrap();
        assert!(new_state.channels.channels.contains_key(&channel_id));
        assert_eq!(
            new_state.channels.channels.get(&channel_id).unwrap().keys,
            vec![verifying_key].into()
        );
    }

    #[test]
    fn test_invalid_signature_error() {
        let ledger_state = LedgerState::new();
        let (_, verifying_key) = create_test_keys_with_seed(1);
        let (wrong_signing_key, _) = create_test_keys_with_seed(2);
        let channel_id = ChannelId::from([4; 32]);

        let blob_op = BlobOp {
            channel: channel_id,
            blob: [42; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let tx = create_signed_tx(Op::ChannelBlob(blob_op), &wrong_signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert_eq!(result, Err(Error::InvalidSignature));
    }

    #[test]
    fn test_unsupported_operation() {
        let ledger_state = LedgerState::new();

        let tx_with_unsupported_op = create_test_tx_with_ops(vec![Op::Native(
            nomos_core::mantle::ops::native::NativeOp {},
        )]);

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx_with_unsupported_op);
        assert_eq!(result, Err(Error::UnsupportedOp));
    }

    #[test]
    fn test_ops_missing_proofs() {
        let ledger_state = LedgerState::new();
        let (_, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([5; 32]);

        let blob_op = BlobOp {
            channel: channel_id,
            blob: [42; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let op = Op::ChannelBlob(blob_op);
        let mut tx = create_test_tx_with_ops(vec![op]);
        tx.ops_profs = vec![None]; // Missing proof

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert_eq!(result, Err(Error::UnsupportedOp));
    }

    #[test]
    fn test_invalid_parent_error() {
        let mut ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([5; 32]);

        // First, create a channel with one message
        let first_blob = BlobOp {
            channel: channel_id,
            blob: [1; 32],
            blob_size: 512,
            da_storage_gas_price: 5,
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let first_tx = create_signed_tx(Op::ChannelBlob(first_blob), &signing_key);
        ledger_state = ledger_state
            .try_apply_tx::<MainnetGasConstants>(first_tx)
            .unwrap();

        // Now try to add a message with wrong parent
        let wrong_parent = MsgId::from([99; 32]);
        let second_blob = BlobOp {
            channel: channel_id,
            blob: [2; 32],
            blob_size: 512,
            da_storage_gas_price: 5,
            parent: wrong_parent,
            signer: verifying_key,
        };

        let second_tx = create_signed_tx(Op::ChannelBlob(second_blob), &signing_key);
        let result = ledger_state
            .clone()
            .try_apply_tx::<MainnetGasConstants>(second_tx);
        assert!(matches!(result, Err(Error::InvalidParent { .. })));

        // Writing into an empty channel with a parent != MsgId::root() should also fail
        let empty_channel_id = ChannelId::from([8; 32]);
        let empty_blob = BlobOp {
            channel: empty_channel_id,
            blob: [3; 32],
            blob_size: 512,
            da_storage_gas_price: 5,
            parent: MsgId::from([1; 32]), // non-root parent
            signer: verifying_key,
        };

        let empty_tx = create_signed_tx(Op::ChannelBlob(empty_blob), &signing_key);
        let empty_result = ledger_state.try_apply_tx::<MainnetGasConstants>(empty_tx);
        assert!(matches!(empty_result, Err(Error::InvalidParent { .. })));
    }

    #[test]
    fn test_unauthorized_signer_error() {
        let mut ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let (unauthorized_signing_key, unauthorized_verifying_key) = create_test_keys_with_seed(3);
        let channel_id = ChannelId::from([6; 32]);

        // First, create a channel with authorized signer
        let first_blob = BlobOp {
            channel: channel_id,
            blob: [1; 32],
            blob_size: 512,
            da_storage_gas_price: 5,
            parent: MsgId::root(),
            signer: verifying_key,
        };

        let correct_parent = first_blob.id();
        let first_tx = create_signed_tx(Op::ChannelBlob(first_blob), &signing_key);
        ledger_state = ledger_state
            .try_apply_tx::<MainnetGasConstants>(first_tx)
            .unwrap();

        // Now try to add a message with unauthorized signer
        let second_blob = BlobOp {
            channel: channel_id,
            blob: [2; 32],
            blob_size: 512,
            da_storage_gas_price: 5,
            parent: correct_parent,
            signer: unauthorized_verifying_key,
        };

        let second_tx = create_signed_tx(Op::ChannelBlob(second_blob), &unauthorized_signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(second_tx);
        assert!(matches!(result, Err(Error::UnauthorizedSigner { .. })));
    }

    #[test]
    fn test_empty_keys_error() {
        let ledger_state = LedgerState::new();
        let (signing_key, _) = create_test_keys();
        let channel_id = ChannelId::from([7; 32]);

        let set_keys_op = SetKeysOp {
            channel: channel_id,
            keys: vec![],
        };

        let tx = create_signed_tx(Op::ChannelSetKeys(set_keys_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert_eq!(result, Err(Error::EmptyKeys { channel_id }));
    }

    #[test]
    fn test_multiple_operations_in_transaction() {
        // Create channel 1 by posting a blob
        // Create channel 2 by posting an inscription
        // Change the keys for channel 1
        // Post another blob in channel 1
        let ledger_state = LedgerState::new();
        let (sk1, vk1) = create_test_keys_with_seed(1);
        let (sk2, vk2) = create_test_keys_with_seed(2);
        let (_, vk3) = create_test_keys_with_seed(3);
        let (sk4, vk4) = create_test_keys_with_seed(4);

        let channel1 = ChannelId::from([10; 32]);
        let channel2 = ChannelId::from([20; 32]);

        let blob_op = BlobOp {
            channel: channel1,
            blob: [42; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::root(),
            signer: vk1,
        };

        let inscribe_op = InscriptionOp {
            channel_id: channel2,
            inscription: vec![1, 2, 3, 4],
            parent: MsgId::root(),
            signer: vk2,
        };

        let set_keys_op = SetKeysOp {
            channel: channel1,
            keys: vec![vk3, vk4],
        };

        let blob_op2 = BlobOp {
            channel: channel1,
            blob: [43; 32],
            blob_size: 2048,
            da_storage_gas_price: 20,
            parent: blob_op.id(),
            signer: vk4,
        };

        let ops = vec![
            Op::ChannelBlob(blob_op),
            Op::ChannelInscribe(inscribe_op),
            Op::ChannelSetKeys(set_keys_op),
            Op::ChannelBlob(blob_op2.clone()),
        ];
        let tx = create_multi_signed_tx(ops, vec![&sk1, &sk2, &sk1, &sk4]);

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(tx);
        assert!(result.is_ok());

        let new_state = result.unwrap();
        assert!(new_state.channels.channels.contains_key(&channel1));
        assert!(new_state.channels.channels.contains_key(&channel2));
        assert_eq!(
            new_state.channels.channels.get(&channel1).unwrap().tip,
            blob_op2.id()
        );
    }
}
