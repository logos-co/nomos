use lb_binary_codec::canonical::{BinaryCodec, BinaryEncode as _};
use lb_groth16::{Fr, serde::serde_fr};
// TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
// use lb_key_management_system_keys::keys::ZkSignature;
use lb_utils::bounded::UpperBoundedVec;
use lb_zktransfer::{MAX_INPUTS, MAX_OUTPUTS, ZkTransferInputsError, ZkTransferVerifierInput};
use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "test-utils"))]
use crate::mantle::notes::{NoteCm, NoteNf};
use crate::{
    block::BlockNumber,
    events::{TxEvent, TxEventPayload},
    mantle::{
        batch::DeferredZkTransfer,
        channel::{Channels, Error},
        gas::{Gas, MainnetGasProfile, OpGasCalculator, OperationGas},
        ledger::{
            ExecutableOperation, PreverifiableOperation, ProvableOperation, Value,
            VerifiableOperation,
            verification_mode::{StandardMode, VerificationMode},
        },
        note_set::{LEDGER_SET, NoteSetError, NoteSetIndex, NoteSets},
        notes::{InputNfs, OutputCms},
        ops::{OpId, SignedOperation, ZkTransferProof, channel::ChannelId},
        transactions::{
            hash::{TxHash, TxHashView},
            states::{Preverified, Unverified, Verified},
        },
    },
};

pub const MAX_METADATA_SIZE: usize = u32::MAX as usize;
pub type Metadata = UpperBoundedVec<u8, { MAX_METADATA_SIZE }>;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, BinaryCodec)]
pub struct DepositOp {
    pub channel_id: ChannelId,
    pub inputs: InputNfs,
    #[serde(with = "serde_fr")]
    pub cm_merkle_root: Fr,
    pub outputs: OutputCms,
    pub excess_value: Value,
    pub metadata: Metadata,
}

impl DepositOp {
    // TODO(RFC-480): the outputs are commitments carried by the payload.
    // // The notes re-created in the channel
    // pub fn outputs(&self, utxos: &Utxos) -> Result<Outputs, Error> {
    //     let notes = self
    //         .inputs
    //         .iter()
    //         .map(|note_id| {
    //             utxos
    //                 .get(note_id)
    //                 .map(|utxo| utxo.note)
    //                 .ok_or(InputsError::InexistingNote(*note_id))
    //         })
    //         .collect::<Result<Vec<_>, _>>()?;
    //
    //     Ok(Outputs::try_new(notes)?)
    // }

    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn sample() -> Self {
        Self {
            channel_id: ChannelId::from([16u8; 32]),
            inputs: [NoteNf(Fr::from(0x11u64))].into(),
            cm_merkle_root: Fr::from(0x26u64),
            outputs: [NoteCm(Fr::from(0x27u64))].into(),
            excess_value: 0x28,
            metadata: Metadata::try_from(b"deposit-metadata".to_vec())
                .expect("Metadata is within bounds."),
        }
    }
}

impl OpId for DepositOp {
    fn op_bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

pub struct DepositValidationContext<'a> {
    pub channels: &'a Channels,
    // TODO(RFC-480): inputs are nullifiers of the ledger set and outputs
    // commitments of the channel set.
    // pub service_notes: &'a ServiceNotes,
    // pub utxos: &'a Utxos,
    pub note_sets: &'a NoteSets,
    pub block_number: BlockNumber,
    pub tx_hash_view: &'a TxHashView,
}

pub struct DepositExecutionContext<'a> {
    // TODO(RFC-480): inputs are nullifiers of the ledger set and outputs
    // commitments of the channel set.
    // pub channels: Channels,
    // pub utxos: Utxos,
    pub channels: &'a Channels,
    pub note_sets: NoteSets,
    pub tx_hash: TxHash,
}

// The index of the note set the channel holds its notes in.
pub(super) fn channel_note_set(
    channels: &Channels,
    channel_id: ChannelId,
) -> Result<NoteSetIndex, Error> {
    channels
        .channel_state(&channel_id)
        .ok_or(Error::ChannelNotFound { channel_id })?
        .note_set
        .ok_or(Error::NoNoteSet { channel_id })
}

impl ProvableOperation for DepositOp {
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // type Proof = ZkSignature;
    type Proof = ZkTransferProof;
    const CODE: u8 = 0x12;
}

impl OperationGas<MainnetGasProfile> for DepositOp {
    const GAS_COST: Gas = Gas::new(619);
}

impl OpGasCalculator<MainnetGasProfile> for DepositOp {}

impl PreverifiableOperation<StandardMode> for SignedOperation<DepositOp, Unverified, StandardMode> {
    type Context<'a> = ();
    type Error = Error;

    fn preverify(&self, _context: &Self::Context<'_>) -> Result<(), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): UTXO deposit tests, inputs are nullifiers of the ledger set
        // and outputs commitments of the channel set.
        // // Ensure the inputs is non-empty
        // self.operation().inputs.preverify()?;

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

impl VerifiableOperation<StandardMode> for SignedOperation<DepositOp, Preverified, StandardMode> {
    type DeferredProof = DeferredZkTransfer;
    type Context<'a> = DepositValidationContext<'a>;
    type Error = Error;

    fn verify(&self, context: &Self::Context<'_>) -> Result<Self::DeferredProof, Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): UTXO deposit tests, inputs are nullifiers of the ledger set
        // and outputs commitments of the channel set.
        // // Check that the channel exists
        // if !context
        //     .channels
        //     .channels
        //     .contains_key(&operation.channel_id)
        // {
        //     return Err(Error::ChannelNotFound {
        //         channel_id: operation.channel_id,
        //     });
        // }
        //
        // // Check that inputs are spendable and not already channel notes
        // operation.inputs.validate_not_in_channel(
        //     context.service_notes,
        //     context.channels,
        //     context.utxos,
        // )?;
        //
        // // Defer the proof verification, so that the caller can batch it.
        // let public_keys = operation.inputs.get_pk(context.utxos)?;
        // let inputs = public_inputs_from_pks((*context.tx_hash_view.as_fr()).into(), &public_keys)
        //     .map_err(|_| Error::InvalidSignature)?;
        // Ok(DeferredZkSig::new(*self.proof().as_proof(), inputs))

        // Check that the channel exists and holds a note set
        channel_note_set(context.channels, operation.channel_id)?;

        // Validate the inputs against the ledger note set
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

impl<Mode: VerificationMode> ExecutableOperation for SignedOperation<DepositOp, Verified, Mode> {
    type Context<'a> = DepositExecutionContext<'a>;
    type Error = Error;

    fn execute<'a>(
        &self,
        mut context: Self::Context<'a>,
    ) -> Result<(Self::Context<'a>, Vec<TxEvent>), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): UTXO deposit tests, inputs are nullifiers of the ledger set
        // and outputs commitments of the channel set.
        // // Get the amount deposited for the event payload
        // let amount_deposited = operation.inputs.amount(&context.utxos)?;
        // let outputs = operation.outputs(&context.utxos)?;
        //
        // // Remove the inputs from the ledger.
        // context.utxos = operation.inputs.execute(context.utxos)?;
        //
        // // Add the re-created notes to the ledger and register them as channel
        // // notes.
        // context.utxos = outputs.execute(context.utxos, operation);
        // let mut notes = DepositRecreatedNotes::default();
        // for utxo in outputs.utxos(operation) {
        //     context.channels = context
        //         .channels
        //         .register_channel_note(&utxo.id(), &operation.channel_id)?;
        //     notes
        //         .try_push(DepositNote {
        //             note_id: utxo.id(),
        //             value: utxo.note.value,
        //             pk: utxo.note.pk,
        //         })
        //         .map_err(InputsError::from)?;
        // }
        //
        // let events = std::iter::once(TxEvent::new(
        //     context.tx_hash,
        //     operation.op_id(),
        //     TxEventPayload::Deposit {
        //         channel_id: operation.channel_id,
        //         amount: amount_deposited,
        //         metadata: operation.metadata.clone(),
        //         notes,
        //     },
        // ))
        // .collect();

        // Resolved again here since the genesis mode executes unverified
        let channel_set = channel_note_set(context.channels, operation.channel_id)?;

        // Spend the inputs in the ledger note set
        let ledger = context
            .note_sets
            .get(LEDGER_SET)
            .expect("the ledger note set always exists")
            .clone()
            .execute_spending(operation.inputs.as_slice())?;

        // Add the outputs to the channel note set
        let channel = context
            .note_sets
            .get(channel_set)
            .expect("the note set of a channel exists")
            .clone()
            .execute_adding(operation.outputs.as_slice())?
            // Step 5 of the Validate of CHANNEL_DEPOSIT, done at execution as
            // for every Operation.
            .buffer_outputs(operation.outputs.as_slice());
        context.note_sets = context
            .note_sets
            .update(LEDGER_SET, ledger)
            .update(channel_set, channel);

        let events = vec![TxEvent::new(
            context.tx_hash,
            operation.op_id(),
            TxEventPayload::Deposit {
                channel_id: operation.channel_id,
                metadata: operation.metadata.clone(),
                outputs: operation.outputs.clone(),
            },
        )];

        Ok((context, events))
    }
}

#[cfg(test)]
mod test {
    use lb_cryptarchia_engine::Slot;
    use lb_groth16::AdditiveGroup as _;
    use lb_key_management_system_keys::keys::Ed25519Key;

    use super::*;
    use crate::mantle::{
        gas::test_utils::FixedThresholds,
        note_set::{CommitmentMmr, NoteSet},
        ops::{
            channel::{NULL_CHANNEL_ID, UnverifiedChannelKeys},
            op_proof::samples::SampleProof as _,
        },
    };

    // TODO(RFC-480): UTXO deposit tests, inputs are nullifiers of the ledger set
    // and outputs commitments of the channel set.
    // #[test]
    // fn preverify_rejects_empty_inputs() {
    //     let deposit = DepositOp {
    //         channel_id: ChannelId::from([0u8; 32]),
    //         inputs: Inputs::empty(),
    //         metadata: Metadata::empty(),
    //     };
    //     let proof = ZkSignature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128]));
    //     let signed_operation = SignedOperation::new(deposit, proof);
    //
    //     assert_eq!(
    //         signed_operation.preverify(&()),
    //         Err(Error::Inputs(InputsError::EmptyInputs))
    //     );
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M4): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_an_unregistered_channel() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //     let (utxos, _) = Utxos::new().insert(input_utxo.id(), input_utxo);
    //
    //     let operation = DepositOp {
    //         inputs: BoundedInputs::from(input_utxo.id()).into(),
    //         ..DepositOp::sample()
    //     };
    //     let channel_id = operation.channel_id;
    //
    //     let signed_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(&[input_key], signed_view.as_fr()).expect("signing should succeed");
    //
    //     let signed_operation =
    //         SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //             .into_preverified(&())
    //             .expect("preverify should accept a non-empty deposit");
    //
    //     let locked_notes = ServiceNotes::new();
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&DepositValidationContext {
    //                 channels: &Channels::new(),
    //                 service_notes: &locked_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &signed_view,
    //             })
    //             .unwrap_err(),
    //         Error::ChannelNotFound { channel_id }
    //     );
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M4): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_an_input_missing_from_the_ledger() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //
    //     let operation = DepositOp {
    //         inputs: BoundedInputs::from(input_utxo.id()).into(),
    //         ..DepositOp::sample()
    //     };
    //
    //     let mut channels = Channels::new();
    //     channels
    //         .channels
    //         .insert_mut(operation.channel_id, make_channel_state(1, None));
    //     let locked_notes = ServiceNotes::new();
    //
    //     let signed_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(&[input_key], signed_view.as_fr()).expect("signing should succeed");
    //
    //     let signed_operation =
    //         SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //             .into_preverified(&())
    //             .expect("preverify should accept a non-empty deposit");
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&DepositValidationContext {
    //                 channels: &channels,
    //                 service_notes: &locked_notes,
    //                 utxos: &Utxos::new(),
    //                 tx_hash_view: &signed_view,
    //             })
    //             .unwrap_err(),
    //         Error::Inputs(InputsError::InexistingNote(input_utxo.id()))
    //     );
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M4): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_an_input_already_owned_by_a_channel() {
    //     let input_key = ZkKey::from(BigUint::from(1u8));
    //     let input_utxo = Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, input_key.to_public_key()),
    //     };
    //     let (utxos, _) = Utxos::new().insert(input_utxo.id(), input_utxo);
    //
    //     let operation = DepositOp {
    //         inputs: BoundedInputs::from(input_utxo.id()).into(),
    //         ..DepositOp::sample()
    //     };
    //
    //     let mut channels = Channels::new();
    //     channels
    //         .channels
    //         .insert_mut(operation.channel_id, make_channel_state(1, None));
    //     let channels = channels
    //         .register_channel_note(&input_utxo.id(), &ChannelId::from([21u8; 32]))
    //         .expect("the note is not owned by another channel");
    //     let locked_notes = ServiceNotes::new();
    //
    //     let signed_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(&[input_key], signed_view.as_fr()).expect("signing should succeed");
    //
    //     let signed_operation =
    //         SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //             .into_preverified(&())
    //             .expect("preverify should accept a non-empty deposit");
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&DepositValidationContext {
    //                 channels: &channels,
    //                 service_notes: &locked_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &signed_view,
    //             })
    //             .unwrap_err(),
    //         Error::Inputs(InputsError::ChannelNote(input_utxo.id()))
    //     );
    // }
    //
    // fn input_utxo(seed: u8) -> Utxo {
    //     Utxo {
    //         op_id: [seed; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, ZkKey::from(BigUint::from(seed)).to_public_key()),
    //     }
    // }
    //
    // fn verified(inputs: Inputs) -> SignedOperation<DepositOp, Verified, StandardMode> {
    //     SignedOperation::<_, Unverified, StandardMode>::new(
    //         DepositOp {
    //             inputs,
    //             ..DepositOp::sample()
    //         },
    //         <DepositOp as ProvableOperation>::Proof::sample(),
    //     )
    //     .into_state_trusted()
    // }
    //
    // #[test]
    // fn execute_recreates_every_input_as_a_channel_note_and_emits_a_deposit_event() {
    //     let first = input_utxo(1);
    //     let second = input_utxo(2);
    //     let (utxos, _) = Utxos::new().insert(first.id(), first);
    //     let (utxos, _) = utxos.insert(second.id(), second);
    //
    //     let signed_operation = verified(
    //         BoundedInputs::try_from_iter([first.id(), second.id()])
    //             .unwrap()
    //             .into(),
    //     );
    //     let operation = signed_operation.operation().clone();
    //     let tx_hash = TxHash::from([9u8; 32]);
    //
    //     let (context, events) = signed_operation
    //         .execute(DepositExecutionContext {
    //             channels: Channels::new(),
    //             utxos,
    //             tx_hash,
    //         })
    //         .expect("both inputs are in the ledger");
    //
    //     let first_deposited = Utxo::new(operation.op_id(), 0, first.note).id();
    //     let second_deposited = Utxo::new(operation.op_id(), 1, second.note).id();
    //     assert!(!context.utxos.contains(&first.id()));
    //     assert!(!context.utxos.contains(&second.id()));
    //     assert!(!context.channels.is_channel_note(&first.id()));
    //     assert!(!context.channels.is_channel_note(&second.id()));
    //     assert!(context.utxos.contains(&first_deposited));
    //     assert!(context.utxos.contains(&second_deposited));
    //     assert!(
    //         context
    //             .channels
    //             .is_channel_note_of(&first_deposited, &operation.channel_id)
    //     );
    //     assert!(
    //         context
    //             .channels
    //             .is_channel_note_of(&second_deposited, &operation.channel_id)
    //     );
    //
    //     assert_eq!(
    //         events,
    //         vec![TxEvent::new(
    //             tx_hash,
    //             operation.op_id(),
    //             TxEventPayload::Deposit {
    //                 channel_id: operation.channel_id,
    //                 amount: first.note.value + second.note.value,
    //                 metadata: operation.metadata,
    //                 notes: DepositRecreatedNotes::try_from(vec![
    //                     DepositNote {
    //                         note_id: first_deposited,
    //                         value: first.note.value,
    //                         pk: first.note.pk,
    //                     },
    //                     DepositNote {
    //                         note_id: second_deposited,
    //                         value: second.note.value,
    //                         pk: second.note.pk,
    //                     },
    //                 ])
    //                 .expect("two recreated notes are within bounds"),
    //             },
    //         )]
    //     );
    // }
    //
    // #[test]
    // fn execute_rejects_an_input_missing_from_the_ledger() {
    //     let input = input_utxo(1);
    //     let signed_operation = verified(BoundedInputs::from(input.id()).into());
    //
    //     assert_eq!(
    //         signed_operation
    //             .execute(DepositExecutionContext {
    //                 channels: Channels::new(),
    //                 utxos: Utxos::new(),
    //                 tx_hash: TxHash::from([9u8; 32]),
    //             })
    //             .map(|_| ())
    //             .map_err(|(_, error)| error),
    //         Err(Error::Inputs(InputsError::InexistingNote(input.id())))
    //     );
    // }
    //
    // #[test]
    // fn execute_rejects_a_recreated_note_another_channel_already_owns() {
    //     let other_channel = ChannelId::from([21u8; 32]);
    //     let input = input_utxo(1);
    //     let (utxos, _) = Utxos::new().insert(input.id(), input);
    //
    //     let signed_operation = verified(BoundedInputs::from(input.id()).into());
    //     let deposited = Utxo::new(signed_operation.operation().op_id(), 0, input.note).id();
    //     let channels = Channels::new()
    //         .register_channel_note(&deposited, &other_channel)
    //         .expect("the recreated note is not owned by another channel yet");
    //
    //     assert_eq!(
    //         signed_operation
    //             .execute(DepositExecutionContext {
    //                 channels,
    //                 utxos,
    //                 tx_hash: TxHash::from([9u8; 32]),
    //             })
    //             .map(|_| ())
    //             .map_err(|(_, error)| error),
    //         Err(Error::ChannelNotes(
    //             channel_notes::Error::AlreadyAChannelNote {
    //                 note_id: deposited,
    //                 channel_id: other_channel,
    //             }
    //         ))
    //     );
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
    //     let operation = DepositOp {
    //         inputs: BoundedInputs::from(input_utxo.id()).into(),
    //         ..DepositOp::sample()
    //     };
    //     let mut channels = Channels::new();
    //     channels
    //         .channels
    //         .insert_mut(operation.channel_id, make_channel_state(1, None));
    //     let tx_hash_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(signers, tx_hash_view.as_fr()).expect("signing should succeed");
    //
    //     SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //         .into_preverified(&())
    //         .expect("preverify should accept a non-empty deposit")
    //         .verify(&DepositValidationContext {
    //             channels: &channels,
    //             service_notes: &ServiceNotes::new(),
    //             utxos: &utxos,
    //             tx_hash_view: &tx_hash_view,
    //         })
    //         .expect("verify leaves the proof to the batch")
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M4): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn deferred_proof_is_accepted() {
    //     assert!(batch_verify(deferred_proof_signed_by(&[input_key()])).is_ok());
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M4): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn wrong_deferred_proof_is_rejected() {
    //     assert!(matches!(
    //         batch_verify(deferred_proof_signed_by(&[unrelated_key()])),
    //         Err(BatchError::InvalidZkSignatures)
    //     ));
    // }

    #[test]
    fn deposit_op_execution_gas_does_not_scale_with_the_threshold() {
        for threshold in [0, 1, 3] {
            assert_eq!(
                DepositOp::sample().execution_gas(&FixedThresholds(threshold)),
                Ok(Gas::new(619))
            );
        }
    }

    const CHANNEL: ChannelId = ChannelId([1u8; 32]);

    fn cm(n: u64) -> NoteCm {
        NoteCm(Fr::from(n))
    }

    fn nf(n: u64) -> NoteNf {
        NoteNf(Fr::from(n))
    }

    fn keys() -> UnverifiedChannelKeys {
        UnverifiedChannelKeys::from(
            Ed25519Key::from_bytes(&[15; 32])
                .public_key()
                .into_unverified(),
        )
    }

    // CHANNEL owns note set 2; the ledger set holds cm(1), recorded at the end of
    // block 0.
    fn state() -> (Channels, NoteSets) {
        let (channels, note_sets) =
            Channels::new().create_channel(CHANNEL, Slot::from(0u64), keys(), NoteSets::new());
        let ledger = NoteSet::empty().execute_adding(&[cm(1)]).unwrap();
        (channels, note_sets.update(LEDGER_SET, ledger).end_block(0))
    }

    fn root() -> Fr {
        state().1.get(LEDGER_SET).unwrap().cm_root()
    }

    fn deposit(inputs: &[NoteNf], outputs: &[NoteCm], cm_merkle_root: Fr) -> DepositOp {
        DepositOp {
            channel_id: CHANNEL,
            inputs: InputNfs::try_from_iter(inputs.iter().copied()).unwrap(),
            cm_merkle_root,
            outputs: OutputCms::try_from_iter(outputs.iter().copied()).unwrap(),
            excess_value: 6,
            metadata: Metadata::empty(),
        }
    }

    fn tx_hash_view() -> TxHashView {
        TxHashView::from(TxHash::from([9u8; 32]))
    }

    fn verify(
        operation: DepositOp,
        channels: &Channels,
        note_sets: &NoteSets,
    ) -> Result<DeferredZkTransfer, Error> {
        SignedOperation::<_, Unverified, StandardMode>::new(operation, ZkTransferProof::sample())
            .into_preverified(&())
            .expect("preverify accepts the deposit")
            .verify(&DepositValidationContext {
                channels,
                note_sets,
                block_number: 1,
                tx_hash_view: &tx_hash_view(),
            })
    }

    #[test]
    fn preverify_rejects_empty_inputs() {
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            deposit(&[], &[cm(2)], root()),
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(Error::NoteSet(NoteSetError::EmptyInputs))
        );
    }

    #[test]
    fn preverify_rejects_more_than_four_inputs() {
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            deposit(&[nf(1), nf(2), nf(3), nf(4), nf(5)], &[], root()),
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(Error::ZkTransferInputs(
                ZkTransferInputsError::TooManyInputs(5)
            ))
        );
    }

    #[test]
    fn verify_rejects_an_unknown_channel() {
        let (_, note_sets) = state();
        assert_eq!(
            verify(
                deposit(&[nf(2)], &[cm(3)], root()),
                &Channels::new(),
                &note_sets
            )
            .unwrap_err(),
            Error::ChannelNotFound {
                channel_id: CHANNEL
            }
        );
    }

    #[test]
    fn verify_rejects_the_null_channel() {
        let (channels, note_sets) = state();
        let (channels, note_sets) =
            channels.create_channel(NULL_CHANNEL_ID, Slot::from(0u64), keys(), note_sets);
        let operation = DepositOp {
            channel_id: NULL_CHANNEL_ID,
            ..deposit(&[nf(2)], &[cm(3)], root())
        };
        assert_eq!(
            verify(operation, &channels, &note_sets).unwrap_err(),
            Error::NoNoteSet {
                channel_id: NULL_CHANNEL_ID
            }
        );
    }

    #[test]
    fn verify_defers_the_zk_transfer_over_the_ledger_set() {
        let (channels, note_sets) = state();
        let deferred = verify(deposit(&[nf(2)], &[cm(3)], root()), &channels, &note_sets).unwrap();
        let public = deferred.inputs();
        assert_eq!(public.inputs[0].into_inner(), Fr::from(2u64));
        assert_eq!(public.inputs[1].into_inner(), Fr::ZERO);
        assert_eq!(public.outputs[0].into_inner(), Fr::from(3u64));
        assert_eq!(public.excess_value.into_inner(), Fr::from(6u64));
        assert_eq!(public.tx_cm_root.into_inner(), root());
        assert_eq!(public.msg.into_inner(), *tx_hash_view().as_fr());
    }

    #[test]
    fn verify_extends_the_ledger_root_with_the_ledger_buffer_only() {
        let (channels, note_sets) = state();
        let ledger = note_sets
            .get(LEDGER_SET)
            .unwrap()
            .clone()
            .buffer_outputs(&[cm(4)]);
        let channel = note_sets.get(2).unwrap().clone().buffer_outputs(&[cm(9)]);
        let note_sets = note_sets.update(LEDGER_SET, ledger).update(2, channel);
        let deferred = verify(deposit(&[nf(2)], &[cm(3)], root()), &channels, &note_sets).unwrap();
        let expected = CommitmentMmr::new()
            .push(cm(1))
            .unwrap()
            .push(cm(4))
            .unwrap()
            .frontier_root();
        assert_eq!(deferred.inputs().tx_cm_root.into_inner(), expected);
    }

    #[test]
    fn verify_rejects_a_spent_ledger_input() {
        let (channels, note_sets) = state();
        let spent = note_sets
            .get(LEDGER_SET)
            .unwrap()
            .clone()
            .execute_spending(&[nf(2)])
            .unwrap();
        let note_sets = note_sets.update(LEDGER_SET, spent);
        assert_eq!(
            verify(deposit(&[nf(2)], &[cm(3)], root()), &channels, &note_sets).unwrap_err(),
            Error::NoteSet(NoteSetError::AlreadySpent(nf(2)))
        );
    }

    #[test]
    fn execute_spends_ledger_inputs_and_adds_the_outputs_to_the_channel_set_and_its_buffer() {
        let (channels, note_sets) = state();
        let operation = deposit(&[nf(2)], &[cm(3)], root());
        let signed_operation: SignedOperation<_, _, StandardMode> =
            SignedOperation::new(operation.clone(), ZkTransferProof::sample()).into_state_trusted();
        let tx_hash = TxHash::from([9u8; 32]);

        let (context, events) = signed_operation
            .execute(DepositExecutionContext {
                channels: &channels,
                note_sets,
                tx_hash,
            })
            .expect("the input is unspent");

        let ledger = context.note_sets.get(LEDGER_SET).unwrap();
        assert!(ledger.contains_nullifier(&nf(2)));
        assert_eq!(ledger.cm_root(), root());
        let channel = context.note_sets.get(2).unwrap();
        let expected = CommitmentMmr::new().push(cm(3)).unwrap().frontier_root();
        assert_eq!(channel.cm_root(), expected);
        assert_eq!(
            channel.tx_cm_root(&CommitmentMmr::new().frontier_root(), 1),
            Ok(expected)
        );
        assert_eq!(
            events,
            vec![TxEvent::new(
                tx_hash,
                operation.op_id(),
                TxEventPayload::Deposit {
                    channel_id: CHANNEL,
                    metadata: Metadata::empty(),
                    outputs: operation.outputs,
                },
            )]
        );
    }
}
