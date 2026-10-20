use lb_binary_codec::canonical::{BinaryCodec, BinaryEncode as _};
use lb_cryptarchia_engine::Slot;
use lb_groth16::{Fr, serde::serde_fr};
// TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
// use lb_key_management_system_keys::keys::ZkSignature;
use lb_zktransfer::{MAX_INPUTS, MAX_OUTPUTS, ZkTransferInputsError, ZkTransferVerifierInput};
use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "test-utils"))]
use crate::mantle::notes::{NoteCm, NoteNf};
use crate::{
    block::BlockNumber,
    events::TxEvent,
    mantle::{
        batch::DeferredZkTransfer,
        channel::{Channels, Error},
        gas::{Gas, MainnetGasProfile, OpGasCalculator, OperationGas},
        ledger::{
            ExecutableOperation, PreverifiableOperation, ProvableOperation, VerifiableOperation,
            verification_mode::{StandardMode, VerificationMode},
        },
        note_set::{LEDGER_SET, NoteSetError, NoteSetIndex, NoteSets},
        notes::{InputNfs, OutputCms},
        ops::{
            OpId, SignedOperation, ZkTransferProof,
            channel::{ChannelId, deposit::channel_note_set},
        },
        transactions::{
            hash::TxHashView,
            states::{Preverified, Unverified, Verified},
        },
    },
};

// ChannelWithdraw = ChannelId InputNfs CmMerkleRoot OutputCms, plain
// field-order concat.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, BinaryCodec)]
pub struct ChannelWithdrawOp {
    pub channel_id: ChannelId,
    pub inputs: InputNfs,
    #[serde(with = "serde_fr")]
    pub cm_merkle_root: Fr,
    pub outputs: OutputCms,
}

impl OpId for ChannelWithdrawOp {
    fn op_bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

pub struct WithdrawValidationContext<'a> {
    pub channels: &'a Channels,
    // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
    // ZkTransfer over the channel note set.
    // pub service_notes: &'a ServiceNotes,
    // pub utxos: &'a Utxos,
    // pub op_index: usize,
    // pub helper: &'a dyn OperationVerificationHelper,
    pub note_sets: &'a NoteSets,
    pub block_number: BlockNumber,
    pub tx_hash_view: &'a TxHashView,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WithdrawExecutionContext<'a> {
    // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
    // ZkTransfer over the channel note set.
    // pub channels: Channels,
    // pub tx_hash: TxHash,
    pub channels: &'a Channels,
    pub pending_withdrawals: PendingWithdrawals,
    pub block_slot: Slot,
}

pub const WITHDRAW_DELAY: u64 = 86_400; // slots

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingWithdrawal {
    pub due: Slot,
    pub note_set: NoteSetIndex,
    pub inputs: InputNfs,
    pub outputs: OutputCms,
}

// Posting order is due order, since every withdrawal of a block has the same
// due slot and slots grow along a chain.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingWithdrawals(rpds::QueueSync<PendingWithdrawal>);

impl PendingWithdrawals {
    #[must_use]
    pub fn new() -> Self {
        Self(rpds::QueueSync::new_sync())
    }

    #[must_use]
    pub fn push(self, withdrawal: PendingWithdrawal) -> Self {
        Self(self.0.enqueue(withdrawal))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PendingWithdrawal> {
        self.0.iter()
    }

    // Applies, in posting order, every withdrawal due at `block_slot` whose
    // inputs are unspent, drops the other due ones, keeps the rest.
    pub fn release(
        self,
        mut note_sets: NoteSets,
        block_slot: Slot,
    ) -> Result<(Self, NoteSets, Vec<PendingWithdrawal>), NoteSetError> {
        let mut queue = self.0;
        let mut released = Vec::new();
        while let Some(withdrawal) = queue.peek().filter(|w| w.due <= block_slot).cloned() {
            queue = queue.dequeue().expect("a peeked queue dequeues");
            let set = note_sets
                .get(withdrawal.note_set)
                .expect("a withdrawal's note set exists")
                .clone();
            // A step consumed one of its notes before it was due: the withdrawal is
            // dropped.
            if withdrawal
                .inputs
                .iter()
                .any(|nf| set.contains_nullifier(nf))
            {
                continue;
            }
            let set = set.execute_spending(withdrawal.inputs.as_slice())?;
            note_sets = note_sets.update(withdrawal.note_set, set);
            let ledger = note_sets
                .get(LEDGER_SET)
                .expect("the ledger note set always exists")
                .clone()
                .execute_adding(withdrawal.outputs.as_slice())?;
            note_sets = note_sets.update(LEDGER_SET, ledger);
            released.push(withdrawal);
        }
        Ok((Self(queue), note_sets, released))
    }
}

impl ProvableOperation for ChannelWithdrawOp {
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // type Proof = ChannelMultiSigProof;
    // type Proof = ZkSignature;
    type Proof = ZkTransferProof;
    const CODE: u8 = 0x13;
}

impl OperationGas<MainnetGasProfile> for ChannelWithdrawOp {
    const GAS_COST: Gas = Gas::new(619);
}

impl OpGasCalculator<MainnetGasProfile> for ChannelWithdrawOp {
    // TODO(RFC-480): transfer_threshold is removed, holders
    // withdraw their own channel notes.
    // fn execution_gas(&self, thresholds: &impl ThresholdSource) -> Result<Gas, GasOverflow> {
    //     Self::GAS_COST.checked_mul(Value::from(thresholds.transfer_threshold(&self.channel_id)))
    // }
}

impl PreverifiableOperation<StandardMode>
    for SignedOperation<ChannelWithdrawOp, Unverified, StandardMode>
{
    type Context<'a> = ();
    type Error = Error;

    fn preverify(&self, _context: &Self::Context<'_>) -> Result<(), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
        // ZkTransfer over the channel note set.
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

impl VerifiableOperation<StandardMode>
    for SignedOperation<ChannelWithdrawOp, Preverified, StandardMode>
{
    type DeferredProof = DeferredZkTransfer;
    type Context<'a> = WithdrawValidationContext<'a>;
    type Error = Error;

    fn verify(&self, context: &Self::Context<'_>) -> Result<Self::DeferredProof, Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
        // ZkTransfer over the channel note set.
        // let proof = self.proof();
        //
        // verify_channel_multi_sig(
        //     &operation.channel_id,
        //     proof,
        //     context.tx_hash_view.as_bytes(),
        //     context.helper,
        //     context.op_index,
        // )
        // .map_err(|_error| Error::InvalidSignature)?; // FIXME: Discards error details
        //
        // // Check that the channel exists
        // let channel =
        //     context
        //         .channels
        //         .channels
        //         .get(&operation.channel_id)
        //         .ok_or(Error::ChannelNotFound {
        //             channel_id: operation.channel_id,
        //         })?;
        //
        // // Check that the inputs are valid and belong to the channel
        // operation.inputs.validate_in_channel(
        //     context.service_notes,
        //     context.channels,
        //     &operation.channel_id,
        //     context.utxos,
        // )?;
        //
        // // Check there is enough signatures
        // let signatures = proof.signatures();
        // if signatures.len() != channel.transfer_threshold as usize {
        //     return Err(Error::ThresholdUnmet {
        //         channel_id: operation.channel_id,
        //         threshold: channel.transfer_threshold,
        //         actual: signatures.len(),
        //     });
        // }
        //
        // // Check the signatures. Don't defer this because ed25519 verification is cheap.
        // for (index, signature) in signatures {
        //     if channel
        //         .accredited_keys
        //         .get(*index as usize)
        //         .ok_or(Error::InvalidSignature)?
        //         .verify(context.tx_hash_view.as_bytes(), signature)
        //         .is_err()
        //     {
        //         return Err(Error::InvalidSignature);
        //     }
        // }
        //
        // Ok(())

        // Check that the channel exists and holds a note set
        let index = channel_note_set(context.channels, operation.channel_id)?;

        // Validate the inputs against the channel note set
        let set = context
            .note_sets
            .get(index)
            .expect("a channel's note set exists");
        set.assert_spendable(
            operation.inputs.as_slice(),
            &operation.cm_merkle_root,
            context.block_number,
        )?;
        let tx_cm_root = set.cached_tx_cm_root(
            context.note_sets.tx_roots(),
            &operation.cm_merkle_root,
            context.block_number,
        )?;

        // Defer the proof verification so that the caller can batch it.
        let nfs: Vec<Fr> = operation.inputs.iter().map(|nf| nf.0).collect();
        let cms: Vec<Fr> = operation.outputs.iter().map(|cm| cm.0).collect();
        // A withdrawal balances exactly
        let public =
            ZkTransferVerifierInput::new(&nfs, &cms, 0, tx_cm_root, *context.tx_hash_view.as_fr())?;
        Ok(DeferredZkTransfer::new(*self.proof().as_proof(), public))
    }
}

impl<Mode: VerificationMode> ExecutableOperation
    for SignedOperation<ChannelWithdrawOp, Verified, Mode>
{
    type Context<'a> = WithdrawExecutionContext<'a>;
    type Error = Error;

    fn execute<'a>(
        &self,
        mut context: Self::Context<'a>,
    ) -> Result<(Self::Context<'a>, Vec<TxEvent>), Self::Error> {
        let operation = self.operation();

        // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
        // ZkTransfer over the channel note set.
        // // Release the inputs from the channel. The notes keep their NoteId,
        // // value and ZkPublicKey and stay in the ledger as regular notes.
        // for note_id in operation.inputs.iter() {
        //     context.channels = context
        //         .channels
        //         .unregister_channel_note(note_id, &operation.channel_id)?;
        // }

        // Resolved again here since the genesis mode executes unverified
        let note_set = channel_note_set(context.channels, operation.channel_id)?;

        // The inputs are spent and the outputs created when the withdrawal is
        // released.
        let due = context.block_slot.strict_add(Slot::from(WITHDRAW_DELAY));
        context.pending_withdrawals = context.pending_withdrawals.push(PendingWithdrawal {
            due,
            note_set,
            inputs: operation.inputs.clone(),
            outputs: operation.outputs.clone(),
        });

        Ok((context, Vec::new()))
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl ChannelWithdrawOp {
    #[must_use]
    pub fn sample() -> Self {
        Self {
            channel_id: ChannelId::from([18u8; 32]),
            inputs: [NoteNf(Fr::from(0x13u64))].into(),
            cm_merkle_root: Fr::from(0x29u64),
            outputs: [NoteCm(Fr::from(0x2au64))].into(),
        }
    }
}

#[cfg(test)]
mod test {
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
        transactions::hash::TxHash,
    };

    // TODO(RFC-480): multi signature withdrawal tests; the holder proves a
    // ZkTransfer over the channel note set.
    // const CHANNEL_ID: ChannelId = ChannelId([18u8; 32]);
    //
    // fn signing_key() -> Ed25519Key {
    //     Ed25519Key::from_bytes(&[0; 32])
    // }
    //
    // fn utxo() -> Utxo {
    //     Utxo {
    //         op_id: [1u8; 32],
    //         output_index: 0,
    //         note: Note::new(10_000, ZkPublicKey::from(Fr::from(1u64))),
    //     }
    // }
    //
    // fn channel_view(register_note: bool) -> Channels {
    //     let mut channels = Channels::new();
    //     channels.channels.insert_mut(
    //         CHANNEL_ID,
    //         make_channel_state(
    //             1,
    //             Some(UnverifiedChannelKeys::new_unchecked(vec![
    //                 signing_key().public_key().into_unverified(),
    //             ])),
    //         ),
    //     );
    //
    //     if register_note {
    //         channels
    //             .register_channel_note(&utxo().id(), &CHANNEL_ID)
    //             .expect("the note is not owned by another channel")
    //     } else {
    //         channels
    //     }
    // }
    //
    // fn ledger_view(transfer_threshold: u16, accredited_keys: UnverifiedChannelKeys) -> Channels {
    //     let mut channels = Channels::new();
    //     channels.channels.insert_mut(
    //         CHANNEL_ID,
    //         make_channel_state(transfer_threshold, Some(accredited_keys)),
    //     );
    //
    //     channels
    //         .register_channel_note(&utxo().id(), &CHANNEL_ID)
    //         .expect("the note is not owned by another channel")
    // }
    //
    // fn preverified(
    //     proof: ChannelMultiSigProof,
    // ) -> SignedOperation<ChannelWithdrawOp, Preverified, StandardMode> {
    //     let operation = ChannelWithdrawOp {
    //         channel_id: CHANNEL_ID,
    //         inputs: BoundedInputs::from(utxo().id()).into(),
    //     };
    //
    //     SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //         .into_preverified(&())
    //         .expect("preverify accepts a non-empty input list")
    // }
    //
    // #[test]
    // fn preverify_rejects_empty_inputs() {
    //     let withdraw = ChannelWithdrawOp {
    //         channel_id: ChannelId::from([0u8; 32]),
    //         inputs: Inputs::empty(),
    //     };
    //     let proof = ChannelMultiSigProof::empty();
    //     let signed_operation = SignedOperation::new(withdraw, proof);
    //
    //     assert_eq!(
    //         signed_operation.preverify(&()),
    //         Err(Error::Inputs(InputsError::EmptyInputs))
    //     );
    // }
    //
    // #[test]
    // fn verify_rejects_signatures_over_another_transaction() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let other_hash = TxHash::from([10u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = channel_view(true);
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     let () = signed_operation
    //         .verify(&WithdrawValidationContext {
    //             channels: &channels,
    //             service_notes: &service_notes,
    //             utxos: &utxos,
    //             tx_hash_view: &TxHashView::from(signed_hash),
    //             op_index: 0,
    //             helper: &helper,
    //         })
    //         .expect("signatures over the signed transaction are accepted");
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &channels,
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(other_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::InvalidSignature
    //     );
    // }
    //
    // #[test]
    // fn verify_rejects_a_signature_the_ledger_view_key_does_not_match() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = ledger_view(
    //         1,
    //         UnverifiedChannelKeys::new_unchecked(vec![
    //             Ed25519Key::from_bytes(&[1; 32])
    //                 .public_key()
    //                 .into_unverified(),
    //         ]),
    //     );
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &channels,
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(signed_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::InvalidSignature
    //     );
    // }
    //
    // #[test]
    // fn verify_rejects_a_signature_index_the_ledger_view_cannot_resolve() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = ledger_view(1, UnverifiedChannelKeys::new_unchecked(vec![]));
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &channels,
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(signed_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::InvalidSignature
    //     );
    // }
    //
    // #[test]
    // fn verify_rejects_a_channel_missing_from_the_ledger_view() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &Channels::new(),
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(signed_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::ChannelNotFound {
    //             channel_id: CHANNEL_ID
    //         }
    //     );
    // }
    //
    // #[test]
    // fn verify_rejects_an_input_the_channel_does_not_own() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = channel_view(false);
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(false),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &channels,
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(signed_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::Inputs(InputsError::NotAChannelNote(utxo().id()))
    //     );
    // }
    //
    // #[test]
    // fn verify_accepts_a_signature_count_meeting_the_ledger_view_threshold() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = ledger_view(
    //         1,
    //         UnverifiedChannelKeys::new_unchecked(vec![
    //             signing_key().public_key().into_unverified(),
    //         ]),
    //     );
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     signed_operation
    //         .verify(&WithdrawValidationContext {
    //             channels: &channels,
    //             service_notes: &service_notes,
    //             utxos: &utxos,
    //             tx_hash_view: &TxHashView::from(signed_hash),
    //             op_index: 0,
    //             helper: &helper,
    //         })
    //         .expect("one accredited signature meets a threshold of one");
    // }
    //
    // #[test]
    // fn verify_rejects_a_signature_count_below_the_ledger_view_threshold() {
    //     let signed_hash = TxHash::from([9u8; 32]);
    //     let signed_operation = preverified(create_channel_multi_sig_proof(
    //         &signed_hash,
    //         &[&signing_key()],
    //     ));
    //
    //     let channels = ledger_view(
    //         2,
    //         UnverifiedChannelKeys::new_unchecked(vec![
    //             signing_key().public_key().into_unverified(),
    //         ]),
    //     );
    //     let helper = TestOperationVerificationHelper::new(
    //         channel_view(true),
    //         [(
    //             (CHANNEL_ID, 0),
    //             signing_key().public_key().into_unverified(),
    //         )],
    //     );
    //     let service_notes = ServiceNotes::new();
    //     let (utxos, _) = Utxos::new().insert(utxo().id(), utxo());
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&WithdrawValidationContext {
    //                 channels: &channels,
    //                 service_notes: &service_notes,
    //                 utxos: &utxos,
    //                 tx_hash_view: &TxHashView::from(signed_hash),
    //                 op_index: 0,
    //                 helper: &helper,
    //             })
    //             .unwrap_err(),
    //         Error::ThresholdUnmet {
    //             channel_id: CHANNEL_ID,
    //             threshold: 2,
    //             actual: 1,
    //         }
    //     );
    // }
    //
    // fn verified() -> SignedOperation<ChannelWithdrawOp, Verified, StandardMode> {
    //     SignedOperation::<_, Unverified, StandardMode>::new(
    //         ChannelWithdrawOp {
    //             channel_id: CHANNEL_ID,
    //             inputs: BoundedInputs::from(utxo().id()).into(),
    //         },
    //         ChannelMultiSigProof::sample_with_signatures(1),
    //     )
    //     .into_state_trusted()
    // }
    //
    // #[test]
    // fn execute_releases_the_inputs_from_the_channel() {
    //     let (context, events) = verified()
    //         .execute(WithdrawExecutionContext {
    //             channels: channel_view(true),
    //             tx_hash: TxHash::from([9u8; 32]),
    //         })
    //         .expect("the input is a channel note of this channel");
    //
    //     assert!(!context.channels.is_channel_note(&utxo().id()));
    //     assert_eq!(events, []);
    // }
    //
    // #[test]
    // fn execute_rejects_an_input_no_channel_owns() {
    //     assert_eq!(
    //         verified()
    //             .execute(WithdrawExecutionContext {
    //                 channels: channel_view(false),
    //                 tx_hash: TxHash::from([9u8; 32]),
    //             })
    //             .map(|_| ())
    //             .map_err(|(_, error)| error),
    //         Err(Error::ChannelNotes(channel_notes::Error::NotInChannel(
    //             utxo().id()
    //         )))
    //     );
    // }
    //
    // #[test]
    // fn execute_rejects_an_input_another_channel_owns() {
    //     let channels = channel_view(false)
    //         .register_channel_note(&utxo().id(), &ChannelId::from([19u8; 32]))
    //         .expect("the note is not owned by another channel yet");
    //
    //     assert_eq!(
    //         verified()
    //             .execute(WithdrawExecutionContext {
    //                 channels,
    //                 tx_hash: TxHash::from([9u8; 32]),
    //             })
    //             .map(|_| ())
    //             .map_err(|(_, error)| error),
    //         Err(Error::ChannelNotes(channel_notes::Error::NotAChannelNote {
    //             note_id: utxo().id(),
    //             channel_id: CHANNEL_ID,
    //         }))
    //     );
    // }
    //
    // #[test]
    // fn channel_withdraw_op_execution_gas_scales_with_the_threshold() {
    //     for (threshold, expected) in [(1, 56), (2, 112), (3, 168)] {
    //         assert_eq!(
    //             ChannelWithdrawOp::sample().execution_gas(&FixedThresholds(threshold)),
    //             Ok(Gas::new(expected))
    //         );
    //     }
    // }
    //
    // #[test]
    // fn channel_withdraw_op_execution_gas_is_zero_against_an_unknown_channel() {
    //     assert_eq!(
    //         ChannelWithdrawOp::sample().execution_gas(&FixedThresholds(0)),
    //         Ok(Gas::new(0))
    //     );
    // }

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

    // CHANNEL owns note set 2, which holds cm(5), recorded at the end of block 0;
    // the ledger set holds cm(1).
    fn state() -> (Channels, NoteSets) {
        let (channels, note_sets) =
            Channels::new().create_channel(CHANNEL, Slot::from(0u64), keys(), NoteSets::new());
        let ledger = NoteSet::empty().execute_adding(&[cm(1)]).unwrap();
        let channel = NoteSet::empty().execute_adding(&[cm(5)]).unwrap();
        (
            channels,
            note_sets
                .update(LEDGER_SET, ledger)
                .update(2, channel)
                .end_block(0),
        )
    }

    fn channel_root() -> Fr {
        CommitmentMmr::new().push(cm(5)).unwrap().frontier_root()
    }

    fn withdrawal(cm_merkle_root: Fr) -> ChannelWithdrawOp {
        ChannelWithdrawOp {
            channel_id: CHANNEL,
            inputs: [nf(5)].into(),
            cm_merkle_root,
            outputs: [cm(50)].into(),
        }
    }

    fn tx_hash_view() -> TxHashView {
        TxHashView::from(TxHash::from([9u8; 32]))
    }

    fn verify(
        operation: ChannelWithdrawOp,
        channels: &Channels,
        note_sets: &NoteSets,
    ) -> Result<DeferredZkTransfer, Error> {
        SignedOperation::<_, Unverified, StandardMode>::new(operation, ZkTransferProof::sample())
            .into_preverified(&())
            .expect("preverify accepts the withdrawal")
            .verify(&WithdrawValidationContext {
                channels,
                note_sets,
                block_number: 1,
                tx_hash_view: &tx_hash_view(),
            })
    }

    #[test]
    fn preverify_rejects_empty_inputs() {
        let operation = ChannelWithdrawOp {
            inputs: InputNfs::default(),
            ..withdrawal(channel_root())
        };
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            operation,
            ZkTransferProof::sample(),
        );
        assert_eq!(
            signed_operation.preverify(&()),
            Err(Error::NoteSet(NoteSetError::EmptyInputs))
        );
    }

    #[test]
    fn verify_rejects_the_null_channel() {
        let (channels, note_sets) = state();
        let (channels, note_sets) =
            channels.create_channel(NULL_CHANNEL_ID, Slot::from(0u64), keys(), note_sets);
        let operation = ChannelWithdrawOp {
            channel_id: NULL_CHANNEL_ID,
            ..withdrawal(channel_root())
        };
        assert_eq!(
            verify(operation, &channels, &note_sets).unwrap_err(),
            Error::NoNoteSet {
                channel_id: NULL_CHANNEL_ID
            }
        );
    }

    #[test]
    fn verify_defers_a_balanced_zk_transfer_over_the_channel_set() {
        let (channels, note_sets) = state();
        let deferred = verify(withdrawal(channel_root()), &channels, &note_sets).unwrap();
        let public = deferred.inputs();
        assert_eq!(public.inputs[0].into_inner(), Fr::from(5u64));
        assert_eq!(public.outputs[0].into_inner(), Fr::from(50u64));
        assert_eq!(public.excess_value.into_inner(), Fr::ZERO);
        assert_eq!(public.tx_cm_root.into_inner(), channel_root());
        assert_eq!(public.msg.into_inner(), *tx_hash_view().as_fr());
    }

    #[test]
    fn verify_rejects_a_root_of_the_ledger_set() {
        let (channels, note_sets) = state();
        let ledger_root = note_sets.get(LEDGER_SET).unwrap().cm_root();
        assert_eq!(
            verify(withdrawal(ledger_root), &channels, &note_sets).unwrap_err(),
            Error::NoteSet(NoteSetError::UnknownRoot(ledger_root))
        );
    }

    #[test]
    fn execute_queues_the_withdrawal_until_its_due_slot() {
        let (channels, _) = state();
        let signed_operation: SignedOperation<_, _, StandardMode> =
            SignedOperation::new(withdrawal(channel_root()), ZkTransferProof::sample())
                .into_state_trusted();

        let (context, events) = signed_operation
            .execute(WithdrawExecutionContext {
                channels: &channels,
                pending_withdrawals: PendingWithdrawals::new(),
                block_slot: Slot::from(10u64),
            })
            .expect("the channel has a note set");

        assert_eq!(
            context.pending_withdrawals.iter().collect::<Vec<_>>(),
            vec![&PendingWithdrawal {
                due: Slot::from(86_410u64),
                note_set: 2,
                inputs: [nf(5)].into(),
                outputs: [cm(50)].into(),
            }]
        );
        assert_eq!(events, []);
    }

    #[test]
    fn channel_withdraw_op_execution_gas_does_not_scale_with_the_threshold() {
        for threshold in [0, 1, 3] {
            assert_eq!(
                ChannelWithdrawOp::sample().execution_gas(&FixedThresholds(threshold)),
                Ok(Gas::new(619))
            );
        }
    }

    fn pending(due: u64, input: u64, output: u64) -> PendingWithdrawal {
        PendingWithdrawal {
            due: Slot::from(due),
            note_set: 2,
            inputs: [nf(input)].into(),
            outputs: [cm(output)].into(),
        }
    }

    #[test]
    fn release_applies_due_withdrawals_with_unspent_inputs_only() {
        let (_, note_sets) = state();
        // A step consumed nf(6) before its withdrawal was due.
        let channel = note_sets
            .get(2)
            .unwrap()
            .clone()
            .execute_spending(&[nf(6)])
            .unwrap();
        let note_sets = note_sets.update(2, channel);
        let ledger_before = note_sets.get(LEDGER_SET).unwrap().clone();
        let withdrawals = PendingWithdrawals::new()
            .push(pending(100, 5, 50))
            .push(pending(100, 6, 60))
            .push(pending(101, 7, 70));

        let (withdrawals, note_sets, released) =
            withdrawals.release(note_sets, Slot::from(100u64)).unwrap();

        assert_eq!(released, vec![pending(100, 5, 50)]);
        assert_eq!(
            withdrawals.iter().collect::<Vec<_>>(),
            vec![&pending(101, 7, 70)]
        );
        let channel = note_sets.get(2).unwrap();
        assert!(channel.contains_nullifier(&nf(5)));
        assert!(!channel.contains_nullifier(&nf(7)));
        let expected = ledger_before.clone().execute_adding(&[cm(50)]).unwrap();
        assert_eq!(
            note_sets.get(LEDGER_SET).unwrap().cm_root(),
            expected.cm_root()
        );
        // No transaction is open, so the released outputs are not buffered.
        assert_eq!(
            note_sets
                .get(LEDGER_SET)
                .unwrap()
                .tx_cm_root(&ledger_before.cm_root(), 1),
            Ok(ledger_before.cm_root())
        );
    }

    #[test]
    fn release_keeps_a_withdrawal_until_its_due_slot() {
        let (_, note_sets) = state();
        let withdrawals = PendingWithdrawals::new().push(pending(100, 5, 50));
        let (withdrawals, note_sets, released) =
            withdrawals.release(note_sets, Slot::from(99u64)).unwrap();
        assert_eq!(released, []);
        assert_eq!(note_sets, state().1);
        let (withdrawals, _, released) =
            withdrawals.release(note_sets, Slot::from(100u64)).unwrap();
        assert_eq!(released, vec![pending(100, 5, 50)]);
        assert_eq!(withdrawals.iter().count(), 0);
    }

    #[test]
    fn release_drops_a_second_withdrawal_of_the_same_note() {
        let (_, note_sets) = state();
        let withdrawals = PendingWithdrawals::new()
            .push(pending(100, 5, 50))
            .push(pending(100, 5, 51));
        let (_, note_sets, released) = withdrawals.release(note_sets, Slot::from(100u64)).unwrap();
        assert_eq!(released, vec![pending(100, 5, 50)]);
        let expected = CommitmentMmr::new()
            .push(cm(1))
            .unwrap()
            .push(cm(50))
            .unwrap()
            .frontier_root();
        assert_eq!(note_sets.get(LEDGER_SET).unwrap().cm_root(), expected);
    }
}
