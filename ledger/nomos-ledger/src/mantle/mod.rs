pub mod channel;
pub mod locked_notes;
pub mod sdp;

use ed25519::signature::Verifier as _;
use nomos_core::{
    block::BlockNumber,
    mantle::{
        ops::{channel::ChannelId, Op, OpProof},
        AuthenticatedMantleTx, GasConstants, NoteId,
    },
    proofs::zksig::{self, ZkSignatureProof as _},
    sdp::state::DeclarationStateError,
};
use sdp::SdpLedgerError;

use crate::{Config, UtxoTree};

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
    #[error("Locked notes error: {0:?}")]
    LockedNotes(#[from] locked_notes::Error),
    #[error("Note not found: {0:?}")]
    NoteNotFound(NoteId),
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
    sdp: sdp::SdpLedger,
    locked_notes: locked_notes::LockedNotes,
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
            sdp: sdp::SdpLedger::new(),
            locked_notes: locked_notes::LockedNotes::new(),
        }
    }

    #[must_use]
    pub const fn locked_notes(&self) -> &locked_notes::LockedNotes {
        &self.locked_notes
    }

    pub fn try_apply_tx<Constants: GasConstants>(
        mut self,
        current_block_number: BlockNumber,
        config: &Config,
        utxo_tree: &UtxoTree,
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
                (
                    Op::SDPDeclare(op),
                    Some(OpProof::ZkAndEd25519Sigs {
                        zk_sig,
                        ed25519_sig,
                    }),
                ) => {
                    let Some((note, _)) = utxo_tree.utxos().get(&op.locked_note_id) else {
                        return Err(Error::NoteNotFound(op.locked_note_id));
                    };
                    if !zk_sig.verify(&zksig::ZkSignaturePublic {
                        pks: vec![note.pk.into(), op.zk_id.0],
                        msg_hash: tx_hash.0,
                    }) {
                        return Err(Error::InvalidSignature);
                    }
                    op.provider_id
                        .0
                        .verify(tx_hash.as_signing_bytes().as_ref(), ed25519_sig)
                        .map_err(|_| Error::InvalidSignature)?;
                    self.locked_notes = self.locked_notes.lock(
                        utxo_tree,
                        &config.min_stake,
                        op.service_type,
                        &op.locked_note_id,
                    )?;
                    self.sdp = self.sdp.apply_declare_msg(current_block_number, op)?;
                }
                (Op::SDPActive(op), Some(OpProof::ZkSig(sig))) => {
                    let declaration = self.sdp.get_declaration(&op.declaration_id)?;
                    let Some((note, _)) = utxo_tree.utxos().get(&declaration.locked_note_id) else {
                        return Err(Error::NoteNotFound(declaration.locked_note_id));
                    };
                    if !sig.verify(&zksig::ZkSignaturePublic {
                        pks: vec![note.pk.into(), declaration.zk_id.0],
                        msg_hash: tx_hash.0,
                    }) {
                        return Err(Error::InvalidSignature);
                    }
                    self.sdp = self.sdp.apply_active_msg(
                        current_block_number,
                        &config.service_params,
                        op,
                    )?;
                }
                (Op::SDPWithdraw(op), Some(OpProof::ZkSig(sig))) => {
                    let declaration = self.sdp.get_declaration(&op.declaration_id)?;
                    let Some((note, _)) = utxo_tree.utxos().get(&declaration.locked_note_id) else {
                        return Err(Error::NoteNotFound(declaration.locked_note_id));
                    };
                    if !sig.verify(&zksig::ZkSignaturePublic {
                        pks: vec![note.pk.into(), declaration.zk_id.0],
                        msg_hash: tx_hash.0,
                    }) {
                        return Err(Error::InvalidSignature);
                    }
                    self.locked_notes = self
                        .locked_notes
                        .unlock(declaration.service_type, &declaration.locked_note_id)?;
                    self.sdp = self.sdp.apply_withdrawn_msg(
                        current_block_number,
                        &config.service_params,
                        op,
                    )?;
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
            ops::{
                channel::{blob::BlobOp, inscribe::InscriptionOp, set_keys::SetKeysOp, MsgId},
                sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
            },
            MantleTx, SignedMantleTx, Transaction as _,
        },
        proofs::zksig::DummyZkSignature,
        sdp::{state::ActiveStateError, ProviderId, ServiceType, ZkPublicKey},
    };
    use num_bigint::BigUint;

    use super::*;
    use crate::cryptarchia::tests::{config, genesis_state, utxo};

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
            ops_proofs: vec![None; mantle_tx.ops.len()],
            ledger_tx_proof: DummyZkSignature::prove(zksig::ZkSignaturePublic {
                pks: vec![],
                msg_hash: mantle_tx.hash().into(),
            }),
            mantle_tx,
        }
    }

    fn create_signed_tx(op: Op, signing_key: &SigningKey) -> SignedMantleTx {
        create_multi_signed_tx(vec![op], vec![signing_key])
    }

    fn create_multi_signed_tx(ops: Vec<Op>, signing_keys: Vec<&SigningKey>) -> SignedMantleTx {
        let mut tx = create_test_tx_with_ops(ops);
        let tx_hash = tx.hash();
        tx.ops_proofs = signing_keys
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
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_channel_inscribe_operation() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert!(result.is_ok());

        let new_state = result.unwrap();
        assert!(new_state.channels.channels.contains_key(&channel_id));
    }

    #[test]
    fn test_channel_set_keys_operation() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
        let ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();
        let channel_id = ChannelId::from([3; 32]);

        let set_keys_op = SetKeysOp {
            channel: channel_id,
            keys: vec![verifying_key],
        };

        let tx = create_signed_tx(Op::ChannelSetKeys(set_keys_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
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
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert_eq!(result, Err(Error::InvalidSignature));
    }

    #[test]
    fn test_unsupported_operation() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
        let ledger_state = LedgerState::new();

        let tx_with_unsupported_op = create_test_tx_with_ops(vec![Op::Native(
            nomos_core::mantle::ops::native::NativeOp {},
        )]);

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx_with_unsupported_op,
        );
        assert_eq!(result, Err(Error::UnsupportedOp));
    }

    #[test]
    fn test_ops_missing_proofs() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
        tx.ops_proofs = vec![None]; // Missing proof

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert_eq!(result, Err(Error::UnsupportedOp));
    }

    #[test]
    fn test_invalid_parent_error() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
            .try_apply_tx::<MainnetGasConstants>(
                0,
                &test_config,
                cryptarchia_state.latest_commitments(),
                first_tx,
            )
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
        let result = ledger_state.clone().try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            second_tx,
        );
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
        let empty_result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            empty_tx,
        );
        assert!(matches!(empty_result, Err(Error::InvalidParent { .. })));
    }

    #[test]
    fn test_unauthorized_signer_error() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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
            .try_apply_tx::<MainnetGasConstants>(
                0,
                &test_config,
                cryptarchia_state.latest_commitments(),
                first_tx,
            )
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
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            second_tx,
        );
        assert!(matches!(result, Err(Error::UnauthorizedSigner { .. })));
    }

    #[test]
    fn test_empty_keys_error() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
        let ledger_state = LedgerState::new();
        let (signing_key, _) = create_test_keys();
        let channel_id = ChannelId::from([7; 32]);

        let set_keys_op = SetKeysOp {
            channel: channel_id,
            keys: vec![],
        };

        let tx = create_signed_tx(Op::ChannelSetKeys(set_keys_op), &signing_key);
        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert_eq!(result, Err(Error::EmptyKeys { channel_id }));
    }

    #[test]
    fn test_multiple_operations_in_transaction() {
        let cryptarchia_state = genesis_state(&[utxo()]);
        let test_config = config();
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

        let result = ledger_state.try_apply_tx::<MainnetGasConstants>(
            0,
            &test_config,
            cryptarchia_state.latest_commitments(),
            tx,
        );
        assert!(result.is_ok());

        let new_state = result.unwrap();
        assert!(new_state.channels.channels.contains_key(&channel1));
        assert!(new_state.channels.channels.contains_key(&channel2));
        assert_eq!(
            new_state.channels.channels.get(&channel1).unwrap().tip,
            blob_op2.id()
        );
    }

    #[test]
    fn test_sdp_withdraw_operation() {
        // First, declare a service to activate.
        let utxo = utxo();
        let cryptarchia_state = genesis_state(&[utxo]);
        let test_config = config();
        let ledger_state = LedgerState::new();
        let (signing_key, verifying_key) = create_test_keys();

        let declare_op = SDPDeclareOp {
            service_type: ServiceType::BlendNetwork,
            locked_note_id: utxo.id(),
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(verifying_key),
            locators: [].into(),
        };

        let declaration_id = declare_op.declaration_id();
        let mut declare_tx = create_test_tx_with_ops(vec![Op::SDPDeclare(declare_op.clone())]);
        let declare_tx_hash = declare_tx.hash();

        declare_tx.ops_proofs = vec![Some(OpProof::ZkAndEd25519Sigs {
            zk_sig: DummyZkSignature::prove(zksig::ZkSignaturePublic {
                pks: vec![utxo.note.pk.into(), declare_op.zk_id.0],
                msg_hash: declare_tx_hash.0,
            }),
            ed25519_sig: signing_key.sign(declare_tx_hash.as_signing_bytes().as_ref()),
        })];

        let ledger_state = ledger_state
            .try_apply_tx::<MainnetGasConstants>(
                0,
                &test_config,
                cryptarchia_state.latest_commitments(),
                declare_tx,
            )
            .unwrap();

        assert!(ledger_state.sdp.get_declaration(&declaration_id).is_ok());
        assert!(ledger_state
            .locked_notes
            .contains(&declare_op.locked_note_id));

        // Apply the active operation.
        let active_op = SDPActiveOp {
            declaration_id,
            nonce: 1,
            metadata: None,
        };
        let mut active_tx = create_test_tx_with_ops(vec![Op::SDPActive(active_op)]);
        let active_tx_hash = active_tx.hash();
        active_tx.ops_proofs = vec![Some(OpProof::ZkSig(DummyZkSignature::prove(
            zksig::ZkSignaturePublic {
                pks: vec![utxo.note.pk.into(), declare_op.zk_id.0],
                msg_hash: active_tx_hash.0,
            },
        )))];

        let ledger_state = ledger_state
            .try_apply_tx::<MainnetGasConstants>(
                1,
                &test_config,
                cryptarchia_state.latest_commitments(),
                active_tx,
            )
            .unwrap();

        let declaration = ledger_state.sdp.get_declaration(&declaration_id).unwrap();
        assert_eq!(declaration.active, 1);
        assert_eq!(declaration.nonce, 1);

        // Apply the withdraw operation.
        let withdraw_op = SDPWithdrawOp {
            declaration_id,
            nonce: 2,
        };
        let mut withdraw_tx = create_test_tx_with_ops(vec![Op::SDPWithdraw(withdraw_op)]);
        let withdraw_tx_hash = withdraw_tx.hash();
        withdraw_tx.ops_proofs = vec![Some(OpProof::ZkSig(DummyZkSignature::prove(
            zksig::ZkSignaturePublic {
                pks: vec![utxo.note.pk.into(), declare_op.zk_id.0],
                msg_hash: withdraw_tx_hash.0,
            },
        )))];

        // Withdrawing a note that is still locked should not be allowed.
        let invalid_ledger_state = ledger_state.clone().try_apply_tx::<MainnetGasConstants>(
            3,
            &test_config,
            cryptarchia_state.latest_commitments(),
            withdraw_tx.clone(),
        );
        assert!(matches!(
            invalid_ledger_state,
            Err(Error::Sdp(SdpLedgerError::SdpStateError(
                DeclarationStateError::Active(ActiveStateError::WithdrawalWhileLocked)
            )))
        ));

        // Withdrawing after lock period is allowed.
        let ledger_state = ledger_state
            .try_apply_tx::<MainnetGasConstants>(
                11,
                &test_config,
                cryptarchia_state.latest_commitments(),
                withdraw_tx,
            )
            .unwrap();

        let declaration = ledger_state.sdp.get_declaration(&declaration_id).unwrap();
        assert!(!ledger_state
            .locked_notes
            .contains(&declare_op.locked_note_id));
        assert_eq!(declaration.active, 1);
        assert_eq!(declaration.withdrawn, Some(11));
        assert_eq!(declaration.nonce, 2);
    }
}
