use lb_binary_codec::canonical::{BinaryCodec, BinaryEncode as _};
use lb_groth16::{AdditiveGroup as _, Fr, serde::serde_fr};
// TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
// use lb_key_management_system_keys::keys::ZkSignature;
use lb_mmr::MerklePath;
use lb_zktransfer::{
    MAX_INPUTS, MAX_OUTPUTS, ZkTransferChainInputsData, ZkTransferInputNoteData,
    ZkTransferInputsError, ZkTransferOutputNoteData, ZkTransferVerifierInput,
    ZkTransferWalletInputsData, ZkTransferWitnessInputsData,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(any(test, feature = "test-utils"))]
use crate::mantle::notes::{NoteCm, NoteNf};
use crate::{
    block::BlockNumber,
    events::TxEvent,
    mantle::{
        batch::DeferredZkTransfer,
        gas::{Gas, MainnetGasProfile, OpGasCalculator, OperationGas},
        ledger::{
            ExecutableOperation, PreverifiableOperation, ProvableOperation, Value,
            VerifiableOperation,
            verification_mode::{StandardMode, VerificationMode},
        },
        note_set::{LEDGER_SET, NoteSetError, NoteSets},
        notes::{InputNfs, Note, OutputCms},
        ops::{OpId, SignedOperation, ZkTransferProof},
        transactions::{
            hash::TxHashView,
            states::{Preverified, Unverified, Verified},
        },
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, BinaryCodec)]
pub struct TransferOp {
    pub inputs: InputNfs,
    pub outputs: OutputCms,
    #[serde(with = "serde_fr")]
    pub cm_merkle_root: Fr,
    pub excess_value: Value,
}

impl TransferOp {
    #[must_use]
    pub const fn new(
        inputs: InputNfs,
        outputs: OutputCms,
        cm_merkle_root: Fr,
        excess_value: Value,
    ) -> Self {
        Self {
            inputs,
            outputs,
            cm_merkle_root,
            excess_value,
        }
    }

    // TODO(RFC-480): inputs are nullifiers and outputs commitments.
    // #[must_use]
    // pub fn is_empty(&self) -> bool {
    //     self.inputs.is_empty() && self.outputs.is_empty()
    // }
    //
    // pub fn utxos(&self) -> impl Iterator<Item = Utxo> {
    //     self.outputs.utxos(self)
    // }
    //
    // #[must_use]
    // pub fn utxo_by_index(&self, index: usize) -> Option<Utxo> {
    //     self.outputs.utxo_by_index(index, self)
    // }
    //
    // pub fn balance(&self, utxos: &Utxos) -> Result<i128, TransferError> {
    //     let mut balance: i128 = 0;
    //     let input_amount = self.inputs.amount(utxos)?;
    //     let output_amount = self.outputs.amount()?;
    //     balance = balance
    //         .checked_add(i128::from(input_amount))
    //         .ok_or(TransferError::BalanceOverflow)?;
    //     balance = balance
    //         .checked_sub(i128::from(output_amount))
    //         .ok_or(TransferError::BalanceOverflow)?;
    //     Ok(balance)
    // }

    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn sample() -> Self {
        Self::new(
            [NoteNf(Fr::from(1u64)), NoteNf(Fr::from(2u64))].into(),
            [NoteCm(Fr::from(3u64)), NoteCm(Fr::from(4u64))].into(),
            Fr::from(5u64),
            6,
        )
    }
}

impl OpId for TransferOp {
    fn op_bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TransferError {
    // TODO(RFC-480): inputs are nullifiers and outputs commitments.
    // #[error("Inputs error: {0}")]
    // Inputs(#[from] ledger::InputsError),
    // #[error("Outputs error: {0}")]
    // Outputs(#[from] ledger::OutputsError),
    // #[error("Applying this transaction would cause a balance overflow")]
    // BalanceOverflow,
    // #[error("Invalid transfer ZkSignature")]
    // InvalidProof,
    #[error(transparent)]
    NoteSet(#[from] NoteSetError),
    #[error(transparent)]
    ZkTransferInputs(#[from] ZkTransferInputsError),
}

pub struct TransferValidationContext<'a> {
    // TODO(RFC-480): inputs are nullifiers and outputs commitments.
    // pub service_notes: &'a ServiceNotes,
    // pub channels: &'a Channels,
    // pub utxos: &'a Utxos,
    pub note_sets: &'a NoteSets,
    pub block_number: BlockNumber,
    pub tx_hash_view: &'a TxHashView,
}

impl ProvableOperation for TransferOp {
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // type Proof = ZkSignature;
    type Proof = ZkTransferProof;
    const CODE: u8 = 0x00;
}

impl OperationGas<MainnetGasProfile> for TransferOp {
    const GAS_COST: Gas = Gas::new(619);
}

impl OpGasCalculator<MainnetGasProfile> for TransferOp {}

impl PreverifiableOperation<StandardMode>
    for SignedOperation<TransferOp, Unverified, StandardMode>
{
    type Context<'a> = ();
    type Error = TransferError;

    fn preverify(&self, _context: &Self::Context<'_>) -> Result<(), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): inputs are nullifiers and outputs commitments.
        // // Ensure the inputs is non-empty
        // operation.inputs.preverify()?;
        //
        // // Validate Outputs
        // operation.outputs.validate()?;

        // Ensure the inputs is non-empty
        if operation.inputs.is_empty() {
            return Err(NoteSetError::EmptyInputs.into());
        }

        // Ensure the ZkTransfer circuit fits the inputs and the outputs
        if operation.inputs.len() > MAX_INPUTS {
            return Err(ZkTransferInputsError::TooManyInputs(operation.inputs.len()).into());
        }
        if operation.outputs.len() > MAX_OUTPUTS {
            return Err(ZkTransferInputsError::TooManyOutputs(operation.outputs.len()).into());
        }

        Ok(())
    }
}

impl VerifiableOperation<StandardMode> for SignedOperation<TransferOp, Preverified, StandardMode> {
    type DeferredProof = DeferredZkTransfer;
    type Context<'a> = TransferValidationContext<'a>;
    type Error = TransferError;

    fn verify(&self, context: &Self::Context<'_>) -> Result<Self::DeferredProof, Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): inputs are nullifiers and outputs commitments.
        // // Validate Inputs
        // operation.inputs.validate_not_in_channel(
        //     context.service_notes,
        //     context.channels,
        //     context.utxos,
        // )?;
        //
        // // Defer the proof verification so that the caller can batch it.
        // let pks = operation.inputs.get_pk(context.utxos)?;
        // let inputs = public_inputs_from_pks((*context.tx_hash_view.as_fr()).into(), &pks)
        //     .map_err(|_| TransferError::InvalidProof)?;
        // Ok(DeferredZkSig::new(*self.proof().as_proof(), inputs))

        // Validate Inputs
        let ledger = context
            .note_sets
            .get(LEDGER_SET)
            .expect("the ledger note set always exists");
        ledger.assert_spendable(
            operation.inputs.as_slice(),
            &operation.cm_merkle_root,
            context.block_number,
        )?;
        let tx_cm_root = ledger.cached_tx_cm_root(
            context.note_sets.tx_roots(),
            &operation.cm_merkle_root,
            context.block_number,
        )?;

        // Defer the proof verification so that the caller can batch it.
        let nfs: Vec<Fr> = operation.inputs.iter().map(|nf| nf.0).collect();
        let cms: Vec<Fr> = operation.outputs.iter().map(|cm| cm.0).collect();
        let public = ZkTransferVerifierInput::new(
            &nfs,
            &cms,
            operation.excess_value,
            tx_cm_root,
            *context.tx_hash_view.as_fr(),
        )?;
        Ok(DeferredZkTransfer::new(*self.proof().as_proof(), public))
    }
}

impl<Mode: VerificationMode> ExecutableOperation for SignedOperation<TransferOp, Verified, Mode> {
    type Context<'a> = NoteSets;
    type Error = TransferError;

    fn execute<'a>(
        &self,
        note_sets: Self::Context<'a>,
    ) -> Result<(Self::Context<'a>, Vec<TxEvent>), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): inputs are nullifiers and outputs commitments.
        // // Remove inputs from the ledger
        // utxos = operation.inputs.execute(utxos)?;
        //
        // // Add outputs from the ledger
        // utxos = operation.outputs.execute(utxos, self.operation());
        // Ok((utxos, Vec::new()))

        // Spend the inputs and add the outputs to the ledger note set
        let ledger = note_sets
            .get(LEDGER_SET)
            .expect("the ledger note set always exists")
            .clone()
            .execute_spending(operation.inputs.as_slice())?
            .execute_adding(operation.outputs.as_slice())?
            // The outputs extend the buffer the next Operations of the
            // transaction are proven against.
            .buffer_outputs(operation.outputs.as_slice());
        Ok((note_sets.update(LEDGER_SET, ledger), Vec::new()))
    }
}

// The witness of a ZkTransfer from note openings and their paths in the
// commitment MMR, the secret keys of the inputs left 0 for the prover to fill.
#[must_use]
pub fn transfer_witness(
    inputs: &[(Note, MerklePath)],
    outputs: &[Note],
    tx_cm_root: Fr,
    msg: Fr,
) -> ZkTransferWitnessInputsData {
    assert!(
        inputs.len() <= MAX_INPUTS,
        "a ZkTransfer spends at most 4 notes"
    );
    assert!(
        outputs.len() <= MAX_OUTPUTS,
        "a ZkTransfer creates at most 8 notes"
    );
    let wallet = ZkTransferWalletInputsData {
        inputs: core::array::from_fn(|i| {
            inputs
                .get(i)
                .map_or_else(ZkTransferInputNoteData::inactive, |(note, path)| {
                    ZkTransferInputNoteData {
                        secret_key: Fr::ZERO,
                        nonce: note.nonce,
                        value: note.value,
                        path_and_selectors: core::array::from_fn(|h| {
                            (path.siblings()[h], (path.leaf_index() >> h) & 1 == 1)
                        }),
                    }
                })
        }),
        outputs: core::array::from_fn(|j| {
            outputs
                .get(j)
                .map_or_else(ZkTransferOutputNoteData::inactive, |note| {
                    ZkTransferOutputNoteData {
                        public_key: *note.public_key.as_fr(),
                        nonce: note.nonce,
                        value: note.value,
                    }
                })
        }),
    };
    let chain = ZkTransferChainInputsData {
        inputs_active: core::array::from_fn(|i| i < inputs.len()),
        outputs_active: core::array::from_fn(|j| j < outputs.len()),
        tx_cm_root,
        msg,
    };
    ZkTransferWitnessInputsData::from_chain_and_wallet_data(chain, wallet)
}

// Proves a ZkTransfer from note openings and their commitment MMR paths, used
// by the genesis builder and tests. Each input is a note, its secret key and
// its path in the commitment MMR.
#[must_use]
pub fn prove_transfer(
    inputs: &[(Note, Fr, MerklePath)],
    outputs: &[Note],
    tx_cm_root: Fr,
    msg: Fr,
) -> lb_zktransfer::ZkTransferProof {
    let openings: Vec<(Note, MerklePath)> = inputs
        .iter()
        .map(|(note, _, path)| (*note, path.clone()))
        .collect();
    let mut witness = transfer_witness(&openings, outputs, tx_cm_root, msg);
    for (input, (_, secret_key, _)) in witness.wallet.inputs.iter_mut().zip(inputs) {
        input.secret_key = *secret_key;
    }
    lb_zktransfer::prove(witness.into())
        .expect("the witness proves")
        .0
}

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    pub use super::prove_transfer;
}

#[cfg(test)]
mod test {
    use lb_groth16::{Field as _, Groth16Input};
    use lb_key_management_system_keys::keys::{ZkKey, ZkPublicKey};

    use super::*;
    use crate::mantle::{
        TxHash,
        gas::test_utils::FixedThresholds,
        note_set::{CommitmentMmr, NoteSet},
        notes::derive_note_nf,
        ops::op_proof::samples::SampleProof as _,
    };

    // TODO(RFC-480): UTXO transfer tests, inputs are now nullifiers and outputs
    // commitments.
    // #[test]
    // fn preverify_rejects_empty_inputs() {
    //     let pk = ZkPublicKey::from(Fr::from(BigUint::from(0u8)));
    //     let transfer = TransferOp {
    //         inputs: Inputs::empty(),
    //         outputs: Outputs::new([Note::new(100, pk)]),
    //     };
    //     let proof = ZkSignature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128]));
    //     let signed_operation = SignedOperation::new(transfer, proof);
    //
    //     assert_eq!(
    //         signed_operation.preverify(&()),
    //         Err(TransferError::Inputs(InputsError::EmptyInputs))
    //     );
    // }
    //
    // #[test]
    // fn preverify_rejects_a_zero_value_output() {
    //     let pk = ZkPublicKey::from(Fr::from(BigUint::from(0u8)));
    //     let transfer = TransferOp {
    //         inputs: BoundedInputs::from(NoteId(Fr::from(BigUint::from(1u8)))).into(),
    //         outputs: Outputs::new([Note::new(0, pk)]),
    //     };
    //     let proof = ZkSignature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128]));
    //     let signed_operation = SignedOperation::new(transfer, proof);
    //
    //     assert_eq!(
    //         signed_operation.preverify(&()),
    //         Err(TransferError::Outputs(ledger::OutputsError::ZeroValueNote))
    //     );
    // }
    //
    // #[test]
    // fn utxos_and_utxo_by_index() {
    //     let pk0 = ZkPublicKey::from(Fr::from(BigUint::from(0u8)));
    //     let pk1 = ZkPublicKey::from(Fr::from(BigUint::from(1u8)));
    //     let pk2 = ZkPublicKey::from(Fr::from(BigUint::from(2u8)));
    //     let transfer = TransferOp {
    //         inputs: BoundedInputs::from(NoteId(BigUint::from(0u8).into())).into(),
    //         outputs: Outputs::new([
    //             Note::new(100, pk0),
    //             Note::new(200, pk1),
    //             Note::new(300, pk2),
    //         ]),
    //     };
    //     assert_eq!(
    //         transfer.utxo_by_index(0),
    //         Some(Utxo {
    //             op_id: transfer.op_id(),
    //             output_index: 0,
    //             note: Note::new(100, pk0),
    //         })
    //     );
    //     assert_eq!(
    //         transfer.utxo_by_index(1),
    //         Some(Utxo {
    //             op_id: transfer.op_id(),
    //             output_index: 1,
    //             note: Note::new(200, pk1),
    //         })
    //     );
    //     assert_eq!(
    //         transfer.utxo_by_index(2),
    //         Some(Utxo {
    //             op_id: transfer.op_id(),
    //             output_index: 2,
    //             note: Note::new(300, pk2),
    //         })
    //     );
    //
    //     assert!(transfer.utxo_by_index(3).is_none());
    // }
    //
    // fn input_key() -> ZkKey {
    //     ZkKey::from(BigUint::from(1u8))
    // }
    //
    // fn unrelated_key() -> ZkKey {
    //     ZkKey::from(BigUint::from(7u8))
    // }
    //
    // fn deferred_proof_signed_by(signers: &[ZkKey]) -> DeferredZkSig {
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key().to_public_key()),
    //     };
    //     let (utxos, _) = Utxos::new().insert(input_utxo.id(), input_utxo);
    //     let operation = TransferOp::new(
    //         BoundedInputs::from(input_utxo.id()).into(),
    //         Outputs::new([Note::new(
    //             10_000,
    //             ZkPublicKey::from(Fr::from(BigUint::from(2u8))),
    //         )]),
    //     );
    //     let tx_hash_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(signers, tx_hash_view.as_fr()).expect("signing should succeed");
    //
    //     SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //         .into_preverified(&())
    //         .expect("preverify should accept a well-formed transfer")
    //         .verify(&TransferValidationContext {
    //             service_notes: &ServiceNotes::new(),
    //             channels: &Channels::new(),
    //             utxos: &utxos,
    //             tx_hash_view: &tx_hash_view,
    //         })
    //         .expect("verify leaves the proof to the batch")
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M3): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn deferred_proof_is_accepted() {
    //     assert!(batch_verify(deferred_proof_signed_by(&[input_key()])).is_ok());
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M3): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn wrong_deferred_proof_is_rejected() {
    //     assert!(matches!(
    //         batch_verify(deferred_proof_signed_by(&[unrelated_key()])),
    //         Err(BatchError::InvalidZkSignatures)
    //     ));
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M3): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_an_input_missing_from_the_ledger() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //
    //     let operation = TransferOp::new(
    //         BoundedInputs::from(input_utxo.id()).into(),
    //         Outputs::new([Note::new(
    //             10_000,
    //             ZkPublicKey::from(Fr::from(BigUint::from(2u8))),
    //         )]),
    //     );
    //
    //     let tx_hash_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(&[input_key], tx_hash_view.as_fr()).expect("signing should succeed");
    //
    //     let signed_operation =
    //         SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //             .into_preverified(&())
    //             .expect("preverify should accept a well-formed transfer");
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&TransferValidationContext {
    //                 service_notes: &ServiceNotes::new(),
    //                 channels: &Channels::new(),
    //                 utxos: &Utxos::new(),
    //                 tx_hash_view: &tx_hash_view,
    //             })
    //             .unwrap_err(),
    //         TransferError::Inputs(InputsError::InexistingNote(input_utxo.id()))
    //     );
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M3): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_an_input_owned_by_a_channel() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //     let (utxos, _) = Utxos::new().insert(input_utxo.id(), input_utxo);
    //
    //     let operation = TransferOp::new(
    //         BoundedInputs::from(input_utxo.id()).into(),
    //         Outputs::new([Note::new(
    //             10_000,
    //             ZkPublicKey::from(Fr::from(BigUint::from(2u8))),
    //         )]),
    //     );
    //
    //     let tx_hash_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(&[input_key], tx_hash_view.as_fr()).expect("signing should succeed");
    //
    //     let signed_operation =
    //         SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //             .into_preverified(&())
    //             .expect("preverify should accept a well-formed transfer");
    //
    //     let channels = Channels::new()
    //         .register_channel_note(&input_utxo.id(), &ChannelId::from([21u8; 32]))
    //         .expect("the note is not owned by another channel");
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&TransferValidationContext {
    //                 service_notes: &ServiceNotes::new(),
    //                 channels: &channels,
    //                 utxos: &utxos,
    //                 tx_hash_view: &tx_hash_view,
    //             })
    //             .unwrap_err(),
    //         TransferError::Inputs(InputsError::ChannelNote(input_utxo.id()))
    //     );
    // }
    //
    // #[test]
    // fn execute_rejects_an_input_missing_from_the_ledger() {
    //     let missing_note = NoteId(Fr::from(BigUint::from(3u8)));
    //     let operation = TransferOp::new(
    //         BoundedInputs::from(missing_note).into(),
    //         Outputs::new([Note::new(
    //             10_000,
    //             ZkPublicKey::from(Fr::from(BigUint::from(2u8))),
    //         )]),
    //     );
    //
    //     let signed_operation: SignedOperation<_, _, StandardMode> = SignedOperation::new(
    //         operation,
    //         <TransferOp as ProvableOperation>::Proof::sample(),
    //     )
    //     .into_state_trusted();
    //
    //     assert_eq!(
    //         signed_operation
    //             .execute(Utxos::new())
    //             .map(|_| ())
    //             .map_err(|(_, error)| error),
    //         Err(TransferError::Inputs(InputsError::InexistingNote(
    //             missing_note
    //         )))
    //     );
    // }
    //
    // #[test]
    // fn execute_removes_the_inputs_and_adds_the_outputs() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //     let (utxos, _) = Utxos::new().insert(input_utxo.id(), input_utxo);
    //
    //     let operation = TransferOp::new(
    //         BoundedInputs::from(input_utxo.id()).into(),
    //         Outputs::new([Note::new(
    //             10_000,
    //             ZkPublicKey::from(Fr::from(BigUint::from(2u8))),
    //         )]),
    //     );
    //     let output_utxo = operation
    //         .utxo_by_index(0)
    //         .expect("the operation declares one output");
    //
    //     let signed_operation: SignedOperation<_, _, StandardMode> = SignedOperation::new(
    //         operation,
    //         <TransferOp as ProvableOperation>::Proof::sample(),
    //     )
    //     .into_state_trusted();
    //
    //     let (utxos, events) = signed_operation
    //         .execute(utxos)
    //         .expect("the input is in the ledger");
    //
    //     assert!(!utxos.contains(&input_utxo.id()));
    //     assert_eq!(utxos.get(&output_utxo.id()), Some(output_utxo));
    //     assert_eq!(events, []);
    // }

    #[test]
    fn transfer_op_execution_gas_does_not_scale_with_the_threshold() {
        for threshold in [0, 1, 3] {
            assert_eq!(
                TransferOp::sample().execution_gas(&FixedThresholds(threshold)),
                Ok(Gas::new(619))
            );
        }
    }

    fn cm(n: u64) -> NoteCm {
        NoteCm(Fr::from(n))
    }

    fn nf(n: u64) -> NoteNf {
        NoteNf(Fr::from(n))
    }

    // The ledger set holds cm(1), recorded at the end of block 0.
    fn note_sets() -> NoteSets {
        let ledger = NoteSet::empty().execute_adding(&[cm(1)]).unwrap();
        NoteSets::new().update(LEDGER_SET, ledger).end_block(0)
    }

    fn root() -> Fr {
        note_sets().get(LEDGER_SET).unwrap().cm_root()
    }

    fn transfer(inputs: &[NoteNf], outputs: &[NoteCm], cm_merkle_root: Fr) -> TransferOp {
        TransferOp::new(
            InputNfs::try_from_iter(inputs.iter().copied()).unwrap(),
            OutputCms::try_from_iter(outputs.iter().copied()).unwrap(),
            cm_merkle_root,
            6,
        )
    }

    fn tx_hash_view() -> TxHashView {
        TxHashView::from(TxHash::from([9u8; 32]))
    }

    fn verify(
        operation: TransferOp,
        note_sets: &NoteSets,
    ) -> Result<DeferredZkTransfer, TransferError> {
        SignedOperation::<_, Unverified, StandardMode>::new(operation, ZkTransferProof::sample())
            .into_preverified(&())
            .expect("preverify accepts the transfer")
            .verify(&TransferValidationContext {
                note_sets,
                block_number: 1,
                tx_hash_view: &tx_hash_view(),
            })
    }

    #[test]
    fn preverify_rejects_empty_inputs() {
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            transfer(&[], &[cm(2)], root()),
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(TransferError::NoteSet(NoteSetError::EmptyInputs))
        );
    }

    #[test]
    fn preverify_rejects_more_than_four_inputs() {
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            transfer(&[nf(1), nf(2), nf(3), nf(4), nf(5)], &[], root()),
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(TransferError::ZkTransferInputs(
                ZkTransferInputsError::TooManyInputs(5)
            ))
        );
    }

    #[test]
    fn preverify_rejects_more_than_eight_outputs() {
        let outputs: Vec<NoteCm> = (1..=9).map(cm).collect();
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            transfer(&[nf(1)], &outputs, root()),
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(TransferError::ZkTransferInputs(
                ZkTransferInputsError::TooManyOutputs(9)
            ))
        );
    }

    #[test]
    fn verify_defers_the_zk_transfer_public_inputs() {
        let deferred = verify(transfer(&[nf(2)], &[cm(3)], root()), &note_sets()).unwrap();
        let public = deferred.inputs();
        assert_eq!(
            public.inputs.map(Groth16Input::into_inner),
            [Fr::from(2u64), Fr::ZERO, Fr::ZERO, Fr::ZERO]
        );
        assert_eq!(
            public.inputs_active.map(Groth16Input::into_inner),
            [Fr::ONE, Fr::ZERO, Fr::ZERO, Fr::ZERO]
        );
        assert_eq!(public.outputs[0].into_inner(), Fr::from(3u64));
        assert_eq!(public.outputs[1].into_inner(), Fr::ZERO);
        assert_eq!(public.excess_value.into_inner(), Fr::from(6u64));
        assert_eq!(public.tx_cm_root.into_inner(), root());
        assert_eq!(public.msg.into_inner(), *tx_hash_view().as_fr());
        assert_eq!(deferred.proof(), ZkTransferProof::sample().as_proof());
    }

    #[test]
    fn verify_extends_the_referenced_root_with_the_buffer() {
        let sets = note_sets();
        let buffered = sets
            .get(LEDGER_SET)
            .unwrap()
            .clone()
            .buffer_outputs(&[cm(4)]);
        let sets = sets.update(LEDGER_SET, buffered);
        let deferred = verify(transfer(&[nf(2)], &[cm(3)], root()), &sets).unwrap();
        let expected = CommitmentMmr::new()
            .push(cm(1))
            .unwrap()
            .push(cm(4))
            .unwrap()
            .frontier_root();
        assert_eq!(deferred.inputs().tx_cm_root.into_inner(), expected);
    }

    #[test]
    fn verify_rejects_an_unknown_root() {
        assert_eq!(
            verify(transfer(&[nf(2)], &[cm(3)], Fr::from(5u64)), &note_sets()).unwrap_err(),
            TransferError::NoteSet(NoteSetError::UnknownRoot(Fr::from(5u64)))
        );
    }

    #[test]
    fn verify_rejects_a_spent_input() {
        let sets = note_sets();
        let spent = sets
            .get(LEDGER_SET)
            .unwrap()
            .clone()
            .execute_spending(&[nf(2)])
            .unwrap();
        let sets = sets.update(LEDGER_SET, spent);
        assert_eq!(
            verify(transfer(&[nf(2)], &[cm(3)], root()), &sets).unwrap_err(),
            TransferError::NoteSet(NoteSetError::AlreadySpent(nf(2)))
        );
    }

    #[test]
    fn execute_spends_adds_and_buffers() {
        let signed_operation: SignedOperation<_, _, StandardMode> = SignedOperation::new(
            transfer(&[nf(2)], &[cm(3)], root()),
            ZkTransferProof::sample(),
        )
        .into_state_trusted();
        let (sets, events) = signed_operation
            .execute(note_sets())
            .expect("the input is unspent");
        let ledger = sets.get(LEDGER_SET).unwrap();
        let expected = CommitmentMmr::new()
            .push(cm(1))
            .unwrap()
            .push(cm(3))
            .unwrap()
            .frontier_root();
        assert!(ledger.contains_nullifier(&nf(2)));
        assert_eq!(ledger.cm_root(), expected);
        assert_eq!(ledger.tx_cm_root(&root(), 1), Ok(expected));
        assert_eq!(events, []);
    }

    #[test]
    fn verify_builds_the_public_inputs_a_real_proof_satisfies() {
        let secret_key = Fr::from(1u64);
        let input = Note::new(
            1_000,
            Fr::from(11u64),
            ZkKey::new(secret_key).to_public_key(),
        );
        let (mmr, path) = CommitmentMmr::new()
            .push_with_paths(input.commitment(), &mut [])
            .unwrap();
        let ledger = NoteSet::empty()
            .execute_adding(&[input.commitment()])
            .unwrap();
        let sets = NoteSets::new().update(LEDGER_SET, ledger).end_block(0);
        let root = sets.get(LEDGER_SET).unwrap().cm_root();
        assert_eq!(root, mmr.frontier_root());
        let output = Note::new(994, Fr::from(12u64), ZkPublicKey::new(Fr::from(2u64)));
        let nullifier = derive_note_nf(&input.commitment(), &secret_key);
        let proof = prove_transfer(
            &[(input, secret_key, path)],
            &[output],
            root,
            *tx_hash_view().as_fr(),
        );

        let operation = TransferOp::new([nullifier].into(), [output.commitment()].into(), root, 6);
        let deferred = SignedOperation::<_, Unverified, StandardMode>::new(
            operation,
            ZkTransferProof::new(proof),
        )
        .into_preverified(&())
        .unwrap()
        .verify(&TransferValidationContext {
            note_sets: &sets,
            block_number: 1,
            tx_hash_view: &tx_hash_view(),
        })
        .unwrap();
        assert!(lb_zktransfer::verify(deferred.proof(), deferred.inputs()).unwrap());

        let overpaid = TransferOp::new([nullifier].into(), [output.commitment()].into(), root, 7);
        let deferred = SignedOperation::<_, Unverified, StandardMode>::new(
            overpaid,
            ZkTransferProof::new(proof),
        )
        .into_preverified(&())
        .unwrap()
        .verify(&TransferValidationContext {
            note_sets: &sets,
            block_number: 1,
            tx_hash_view: &tx_hash_view(),
        })
        .unwrap();
        assert!(!lb_zktransfer::verify(deferred.proof(), deferred.inputs()).unwrap());
    }

    #[test]
    fn transfer_witness_leaves_the_secret_keys_to_the_prover() {
        let input = Note::new(1_000, Fr::from(11u64), ZkPublicKey::new(Fr::from(2u64)));
        let (mmr, path) = CommitmentMmr::new()
            .push(NoteCm(Fr::from(4u64)))
            .unwrap()
            .push_with_paths(input.commitment(), &mut [])
            .unwrap();
        let output = Note::new(994, Fr::from(12u64), ZkPublicKey::new(Fr::from(3u64)));
        let witness = transfer_witness(
            &[(input, path)],
            &[output],
            mmr.frontier_root(),
            Fr::from(77u64),
        );
        assert_eq!(witness.wallet.inputs[0].secret_key, Fr::ZERO);
        assert_eq!(
            witness.wallet.inputs[0].path_and_selectors[0],
            (Fr::from(4u64), true)
        );
        assert_eq!(witness.chain.inputs_active, [true, false, false, false]);
        assert_eq!(
            witness.chain.outputs_active,
            [true, false, false, false, false, false, false, false]
        );
        assert_eq!(
            (witness.chain.tx_cm_root, witness.chain.msg),
            (mmr.frontier_root(), Fr::from(77u64))
        );
    }
}
