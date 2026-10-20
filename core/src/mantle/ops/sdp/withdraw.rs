use lb_cryptarchia_engine::Epoch;
use lb_groth16::Fr;
// TODO(RFC-480): the service note is consumed by its nullifier against the root of its
// commitment.
// use lb_key_management_system_keys::keys::{ZkSignature, public_inputs_from_pks};
use lb_log_targets::mantle;
use lb_zktransfer::ZkTransferVerifierInput;
use tracing::debug;

use super::{SDPWithdrawOp, SdpError};
// TODO(RFC-480): the service note is consumed by its nullifier against the root of its
// commitment. `batch::DeferredZkSig` and `sdp::service_notes::ServiceNotes` served the
// locked note.
use crate::{
    events::TxEvent,
    mantle::{
        batch::DeferredZkTransfer,
        gas::{Gas, MainnetGasProfile, OpGasCalculator, OperationGas},
        ledger::{
            Declarations, ExecutableOperation, PreverifiableOperation, ProvableOperation,
            VerifiableOperation,
            verification_mode::{StandardMode, VerificationMode},
        },
        note_set::{CommitmentMmr, NoteSetError, NoteSets, SDP_SET},
        ops::{SignedOperation, ZkAndEd25519Proof},
        transactions::{
            hash::TxHashView,
            states::{Preverified, Unverified, Verified},
        },
    },
    sdp,
};

const LOG_TARGET: &str = mantle::sdp::message::WITHDRAW;

pub struct SDPWithdrawValidationContext<'a> {
    pub declarations: &'a Declarations,
    pub epoch: Epoch,
    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // pub service_notes: &'a ServiceNotes,
    pub note_sets: &'a NoteSets,
    pub tx_hash_view: &'a TxHashView,
}

pub struct SDPWithdrawExecutionContext {
    pub declarations: Declarations,
    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // pub service_notes: ServiceNotes,
    pub note_sets: NoteSets,
    pub epoch: Epoch,
}

impl ProvableOperation for SDPWithdrawOp {
    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // type Proof = ZkSignature;
    type Proof = ZkAndEd25519Proof;
    const CODE: u8 = 0x21;
}

impl OperationGas<MainnetGasProfile> for SDPWithdrawOp {
    const GAS_COST: Gas = Gas::new(678);
}

impl OpGasCalculator<MainnetGasProfile> for SDPWithdrawOp {}

impl PreverifiableOperation<StandardMode>
    for SignedOperation<SDPWithdrawOp, Unverified, StandardMode>
{
    type Context<'a> = ();
    type Error = SdpError;

    fn preverify(&self, _context: &Self::Context<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl VerifiableOperation<StandardMode>
    for SignedOperation<SDPWithdrawOp, Preverified, StandardMode>
{
    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // type DeferredProof = DeferredZkSig;
    type DeferredProof = DeferredZkTransfer;
    type Context<'a> = SDPWithdrawValidationContext<'a>;
    type Error = SdpError;

    fn verify(&self, context: &Self::Context<'_>) -> Result<Self::DeferredProof, Self::Error> {
        let operation = self.operation();

        // Check that the declaration exists
        let Some(declaration) = context.declarations.get(&operation.declaration_id) else {
            return Err(SdpError::DeclarationNotFound(operation.declaration_id));
        };

        // Check that the declaration hasn't been already scheduled to be withdrawn.
        if let Some(withdraw_at) = declaration.withdraw_at {
            return Err(SdpError::DeclarationWithdrawn {
                declaration_id: operation.declaration_id,
                withdraw_at,
            });
        }

        // Ensure the `provider_id` attached to this declaration authorized this Operation.
        declaration
            .provider_id
            .0
            .verify(context.tx_hash_view.as_bytes(), &self.proof().ed25519_sig)
            .map_err(|_| SdpError::InvalidEddsaSignature)?;

        // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
        // commitment.
        // // Check that the service note is used for this service
        // if !context
        //     .service_notes
        //     .is_used_for_service(&operation.service_note_id, &declaration.service_type)
        // {
        //     return Err(SdpError::NoteNotUsedForService {
        //         note_id: operation.service_note_id,
        //         service_type: declaration.service_type,
        //     });
        // }
        //
        // // TODO(RFC-480 M5 Task 3): the withdrawal is proven against the root of its service
        // // note.
        // // // Check that the service note exists (it corresponds to the declaration service
        // // // note)
        // // if declaration.service_note_id != operation.service_note_id {
        // //     return Err(SdpError::InvalidServiceNote {
        // //         note_id: operation.service_note_id,
        // //         expected: declaration.service_note_id,
        // //     });
        // // }

        // Check that the nonce is greater than the previous one
        if operation.nonce <= declaration.nonce {
            return Err(SdpError::InvalidNonce {
                message_nonce: operation.nonce,
                declaration_nonce: declaration.nonce,
            });
        }

        // Check that the service note is not spent
        if context
            .note_sets
            .get(SDP_SET)
            .expect("the SDP note set always exists")
            .contains_nullifier(&operation.service_note_nf)
        {
            return Err(NoteSetError::AlreadySpent(operation.service_note_nf).into());
        }

        // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
        // commitment.
        // // Defer the proof verification so that the caller can batch it.
        // // Ensure service note pk and zk_id attached to this declaration authorized this
        // // Operation.
        // let note = context
        //     .service_notes
        //     .get(&operation.service_note_id)
        //     .expect("The Operation has been checked above");
        // let inputs = public_inputs_from_pks(
        //     (*context.tx_hash_view.as_fr()).into(),
        //     &[note.pk, declaration.zk_id],
        // )
        // .map_err(|_| SdpError::InvalidZkSignature)?;
        //
        // Ok(DeferredZkSig::new(*self.proof().as_proof(), inputs))

        // The root of an empty MMR to which only the service note of the declaration is
        // appended.
        let tx_cm_root = CommitmentMmr::new()
            .push(declaration.service_note)
            .expect("one commitment fits in an empty MMR")
            .frontier_root();

        // Defer the proof verification so that the caller can batch it.
        let cms: Vec<Fr> = operation.outputs.iter().map(|cm| cm.0).collect();
        let public = ZkTransferVerifierInput::new(
            &[operation.service_note_nf.0],
            &cms,
            operation.excess_value,
            tx_cm_root,
            *context.tx_hash_view.as_fr(),
        )?;
        Ok(DeferredZkTransfer::new(
            *self.proof().zk_proof.as_proof(),
            public,
        ))
    }
}

impl<Mode: VerificationMode> ExecutableOperation
    for SignedOperation<SDPWithdrawOp, Verified, Mode>
{
    type Context<'a> = SDPWithdrawExecutionContext;
    type Error = SdpError;

    fn execute<'a>(
        &self,
        mut context: Self::Context<'a>,
    ) -> Result<(Self::Context<'a>, Vec<TxEvent>), Self::Error> {
        let operation = self.operation();

        // Spend the service note from the SDP set
        let sdp = context
            .note_sets
            .get(SDP_SET)
            .expect("the SDP note set always exists")
            .clone()
            .execute_spending(&[operation.service_note_nf])?;
        context.note_sets = context.note_sets.update(SDP_SET, sdp);

        let declaration = context
            .declarations
            .get_mut(&operation.declaration_id)
            .expect("The operation should have been validated");

        // Delay the withdrawal by `SNAPSHOT_FINALIZATION_DELAY` epochs
        // to prevent "stake-less service provision".
        // Otherwise, providers can continue providing the service even after
        // withdrawal because SDP uses the snapshot from `SNAPSHOT_FINALIZATION_DELAY`
        // epochs ago.
        // The note will be unlocked once the withdrawn epoch set here is reached.
        declaration.withdraw_at = Some(context.epoch.strict_add(sdp::SNAPSHOT_FINALIZATION_DELAY));
        declaration.nonce = operation.nonce;
        // Released to the ledger set when the declaration is removed.
        declaration.withdraw_outputs = operation.outputs.clone();

        debug!(
            target: LOG_TARGET,
            provider_id = ?declaration.provider_id,
            withdraw_at = ?declaration.withdraw_at,
            nonce = ?declaration.nonce,
            "updated declaration with withdraw message"
        );

        Ok((context, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use lb_groth16::{AdditiveGroup as _, Groth16Input};
    use lb_key_management_system_keys::keys::{Ed25519Key, ZkKey, ZkPublicKey};
    use lb_zktransfer::ZkTransferInputsError;

    use super::*;
    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment. `BigUint`, `NoteId`, `BatchError`, `batch_verify`, `ledger::Note`,
    // `MinStake` and `ServiceType` served the locked note tests.
    use crate::{
        mantle::{
            TxHash,
            gas::test_utils::FixedThresholds,
            note_set::{LEDGER_SET, NoteSet},
            notes::{self, NoteCm, NoteNf, OutputCms, derive_note_nf},
            ops::{
                ZkTransferProof, op_proof::samples::SampleProof as _,
                transfer::test_utils::prove_transfer,
            },
        },
        sdp::{Declaration, DeclarationMessage},
    };

    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // fn note_key() -> ZkKey {
    //     ZkKey::from(BigUint::from(1u8))
    // }
    //
    // fn declaration_key() -> ZkKey {
    //     ZkKey::from(BigUint::from(2u8))
    // }
    //
    // fn locked_notes(service_note_id: &NoteId) -> ServiceNotes {
    //     ServiceNotes::new()
    //         .lock(
    //             &MinStake {
    //                 threshold: 0,
    //                 timestamp: 0,
    //             },
    //             ServiceType::BlendNetwork,
    //             Note::new(10_000, note_key().to_public_key()),
    //             service_note_id,
    //         )
    //         .expect("the note covers the minimum stake")
    // }

    // The provider of `DeclarationMessage::sample()`.
    fn provider_key() -> Ed25519Key {
        Ed25519Key::from_bytes(&[24; 32])
    }

    fn tx_hash_view() -> TxHashView {
        TxHashView::from(TxHash::from([9u8; 32]))
    }

    fn declaration() -> Declaration {
        Declaration::new(Epoch::from(0), &DeclarationMessage::sample())
    }

    fn service_note_root(message: &DeclarationMessage) -> Fr {
        CommitmentMmr::new()
            .push(message.service_note().commitment())
            .unwrap()
            .frontier_root()
    }

    fn declarations(operation: &SDPWithdrawOp, declaration: Declaration) -> Declarations {
        Declarations::new_sync().insert(operation.declaration_id, declaration)
    }

    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // fn preverified(
    //     operation: SDPWithdrawOp,
    //     tx_hash_view: &TxHashView,
    // ) -> SignedOperation<SDPWithdrawOp, Preverified, StandardMode> {
    //     let proof = ZkKey::multi_sign(&[note_key(), declaration_key()], tx_hash_view.as_fr())
    //         .expect("signing should succeed");
    //
    //     SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //         .into_preverified(&())
    //         .expect("preverify accepts every withdraw message")
    // }

    fn preverified(
        operation: SDPWithdrawOp,
        zk_proof: ZkTransferProof,
        signing_key: &Ed25519Key,
        signed_view: &TxHashView,
    ) -> SignedOperation<SDPWithdrawOp, Preverified, StandardMode> {
        SignedOperation::<_, Unverified, StandardMode>::new(
            operation,
            ZkAndEd25519Proof::new(
                zk_proof,
                signing_key.sign_payload(signed_view.as_bytes().as_ref()),
            ),
        )
        .into_preverified(&())
        .expect("preverify accepts every withdraw message")
    }

    fn verify_withdrawal(
        signed_operation: &SignedOperation<SDPWithdrawOp, Preverified, StandardMode>,
        declarations: &Declarations,
        note_sets: &NoteSets,
    ) -> Result<DeferredZkTransfer, SdpError> {
        signed_operation.verify(&SDPWithdrawValidationContext {
            declarations,
            epoch: Epoch::from(0),
            note_sets,
            tx_hash_view: &tx_hash_view(),
        })
    }

    fn verify_sample(
        operation: SDPWithdrawOp,
        declarations: &Declarations,
    ) -> Result<DeferredZkTransfer, SdpError> {
        verify_withdrawal(
            &preverified(
                operation,
                ZkTransferProof::sample(),
                &provider_key(),
                &tx_hash_view(),
            ),
            declarations,
            &NoteSets::new(),
        )
    }

    #[test]
    fn preverify_accepts_every_withdraw_message() {
        let signed_operation = SignedOperation::<_, Unverified, StandardMode>::new(
            SDPWithdrawOp::sample(),
            <SDPWithdrawOp as ProvableOperation>::Proof::sample(),
        );

        assert_eq!(signed_operation.preverify(&()), Ok(()));
    }

    #[test]
    fn verify_rejects_an_unknown_declaration() {
        let operation = SDPWithdrawOp::sample();
        let declaration_id = operation.declaration_id;
        let declarations = Declarations::new_sync();

        assert_eq!(
            verify_sample(operation, &declarations).unwrap_err(),
            SdpError::DeclarationNotFound(declaration_id)
        );
    }

    #[test]
    fn verify_rejects_a_declaration_already_scheduled_for_withdrawal() {
        let operation = SDPWithdrawOp::sample();
        let declaration_id = operation.declaration_id;

        let withdraw_at = Epoch::from(7);
        let declaration = Declaration {
            withdraw_at: Some(withdraw_at),
            ..declaration()
        };
        let declarations = declarations(&operation, declaration);

        assert_eq!(
            verify_sample(operation, &declarations).unwrap_err(),
            SdpError::DeclarationWithdrawn {
                declaration_id,
                withdraw_at,
            }
        );
    }

    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // #[test]
    // #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_a_note_not_locked_for_the_service() {
    //     let operation = SDPWithdrawOp::sample();
    //     let note_id = operation.service_note_id;
    //     let service_notes = ServiceNotes::new();
    //     let declarations = declarations(&operation, declaration(operation.service_note_id));
    //
    //     let signed_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let signed_operation = preverified(operation, &signed_view);
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&SDPWithdrawValidationContext {
    //                 declarations: &declarations,
    //                 epoch: Epoch::from(0),
    //                 service_notes: &service_notes,
    //                 tx_hash_view: &signed_view,
    //             })
    //             .unwrap_err(),
    //         SdpError::NoteNotUsedForService {
    //             note_id,
    //             service_type: ServiceType::BlendNetwork,
    //         }
    //     );
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn verify_rejects_a_locked_note_the_declaration_does_not_own() {
    //     let operation = SDPWithdrawOp::sample();
    //     let note_id = operation.service_note_id;
    //     let expected = NoteId(Fr::from(99u64));
    //     let service_notes = locked_notes(&note_id);
    //     let declarations = declarations(&operation, declaration(expected));
    //
    //     let signed_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let signed_operation = preverified(operation, &signed_view);
    //
    //     assert_eq!(
    //         signed_operation
    //             .verify(&SDPWithdrawValidationContext {
    //                 declarations: &declarations,
    //                 epoch: Epoch::from(0),
    //                 service_notes: &service_notes,
    //                 tx_hash_view: &signed_view,
    //             })
    //             .unwrap_err(),
    //         SdpError::InvalidServiceNote { note_id, expected }
    //     );
    // }
    //
    // fn deferred_proof_signed_by(signers: &[ZkKey]) -> DeferredZkSig {
    //     let operation = SDPWithdrawOp::sample();
    //     let service_notes = locked_notes(&operation.service_note_id);
    //     let declarations = declarations(&operation, declaration(operation.service_note_id));
    //     let tx_hash_view = TxHashView::from(TxHash::from([9u8; 32]));
    //     let proof =
    //         ZkKey::multi_sign(signers, tx_hash_view.as_fr()).expect("signing should succeed");
    //
    //     SignedOperation::<_, Unverified, StandardMode>::new(operation, proof)
    //         .into_preverified(&())
    //         .expect("preverify accepts every withdraw message")
    //         .verify(&SDPWithdrawValidationContext {
    //             declarations: &declarations,
    //             epoch: Epoch::from(0),
    //             service_notes: &service_notes,
    //             tx_hash_view: &tx_hash_view,
    //         })
    //         .expect("verify leaves the proof to the batch")
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn deferred_proof_is_accepted() {
    //     assert!(batch_verify(deferred_proof_signed_by(&[note_key(), declaration_key()])).is_ok());
    // }
    //
    // #[test]
    // #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    // fn wrong_deferred_proof_is_rejected() {
    //     assert!(matches!(
    //         batch_verify(deferred_proof_signed_by(&[declaration_key()])),
    //         Err(BatchError::InvalidZkSignatures)
    //     ));
    // }

    #[test]
    fn verify_rejects_a_nonce_that_does_not_increase() {
        let operation = SDPWithdrawOp {
            nonce: 0,
            ..SDPWithdrawOp::sample()
        };
        let declarations = declarations(&operation, declaration());

        assert_eq!(
            verify_sample(operation, &declarations).unwrap_err(),
            SdpError::InvalidNonce {
                message_nonce: 0,
                declaration_nonce: 0,
            }
        );
    }

    #[test]
    fn verify_defers_the_zk_transfer_against_the_service_note_root() {
        let operation = SDPWithdrawOp::sample();
        let declarations = declarations(&operation, declaration());
        // Other notes of the SDP set do not change the root the withdrawal is proven against.
        let sdp = NoteSet::empty()
            .execute_adding(&[NoteCm(Fr::from(1u64))])
            .unwrap();
        let sets = NoteSets::new().update(SDP_SET, sdp).end_block(0);
        let deferred = verify_withdrawal(
            &preverified(
                operation,
                ZkTransferProof::sample(),
                &provider_key(),
                &tx_hash_view(),
            ),
            &declarations,
            &sets,
        )
        .unwrap();
        let public = deferred.inputs();
        assert_eq!(
            public.inputs.map(Groth16Input::into_inner),
            [Fr::from(28u64), Fr::ZERO, Fr::ZERO, Fr::ZERO]
        );
        assert_eq!(public.outputs[0].into_inner(), Fr::from(46u64));
        assert_eq!(public.outputs[1].into_inner(), Fr::ZERO);
        assert_eq!(public.excess_value.into_inner(), Fr::from(47u64));
        assert_eq!(
            public.tx_cm_root.into_inner(),
            service_note_root(&DeclarationMessage::sample())
        );
        assert_eq!(public.msg.into_inner(), *tx_hash_view().as_fr());
    }

    #[test]
    fn verify_rejects_a_signature_by_another_key() {
        let operation = SDPWithdrawOp::sample();
        let declarations = declarations(&operation, declaration());
        assert_eq!(
            verify_withdrawal(
                &preverified(
                    operation,
                    ZkTransferProof::sample(),
                    &Ed25519Key::from_bytes(&[25; 32]),
                    &tx_hash_view()
                ),
                &declarations,
                &NoteSets::new(),
            )
            .unwrap_err(),
            SdpError::InvalidEddsaSignature
        );
    }

    #[test]
    fn verify_rejects_a_spent_service_note() {
        let operation = SDPWithdrawOp::sample();
        let declarations = declarations(&operation, declaration());
        let spent = NoteSet::empty()
            .execute_spending(&[NoteNf(Fr::from(28u64))])
            .unwrap();
        let sets = NoteSets::new().update(SDP_SET, spent);
        assert_eq!(
            verify_withdrawal(
                &preverified(
                    operation,
                    ZkTransferProof::sample(),
                    &provider_key(),
                    &tx_hash_view()
                ),
                &declarations,
                &sets,
            )
            .unwrap_err(),
            SdpError::NoteSet(NoteSetError::AlreadySpent(NoteNf(Fr::from(28u64))))
        );
    }

    #[test]
    fn verify_rejects_more_than_eight_outputs() {
        let operation = SDPWithdrawOp {
            outputs: OutputCms::try_from_iter((1..=9u64).map(|n| NoteCm(Fr::from(n)))).unwrap(),
            ..SDPWithdrawOp::sample()
        };
        let declarations = declarations(&operation, declaration());
        assert_eq!(
            verify_sample(operation, &declarations).unwrap_err(),
            SdpError::ZkTransferInputs(ZkTransferInputsError::TooManyOutputs(9))
        );
    }

    #[test]
    fn verify_builds_the_public_inputs_a_real_proof_satisfies() {
        let secret_key = Fr::from(3u64);
        let message = DeclarationMessage {
            zk_id: ZkKey::new(secret_key).to_public_key(),
            ..DeclarationMessage::sample()
        };
        let service_note = message.service_note();
        let (mmr, path) = CommitmentMmr::new()
            .push_with_paths(service_note.commitment(), &mut [])
            .unwrap();
        let output = notes::Note::new(40, Fr::from(12u64), ZkPublicKey::new(Fr::from(2u64)));
        let withdrawal = |excess_value| SDPWithdrawOp {
            declaration_id: message.id(),
            nonce: 1,
            service_note_nf: derive_note_nf(&service_note.commitment(), &secret_key),
            outputs: [output.commitment()].into(),
            excess_value,
        };
        let declarations = Declarations::new_sync()
            .insert(message.id(), Declaration::new(Epoch::from(0), &message));
        let proof = prove_transfer(
            &[(service_note, secret_key, path)],
            &[output],
            mmr.frontier_root(),
            *tx_hash_view().as_fr(),
        );

        let deferred = verify_withdrawal(
            &preverified(
                withdrawal(5),
                ZkTransferProof::new(proof),
                &provider_key(),
                &tx_hash_view(),
            ),
            &declarations,
            &NoteSets::new(),
        )
        .unwrap();
        assert!(lb_zktransfer::verify(deferred.proof(), deferred.inputs()).unwrap());

        let deferred = verify_withdrawal(
            &preverified(
                withdrawal(6),
                ZkTransferProof::new(proof),
                &provider_key(),
                &tx_hash_view(),
            ),
            &declarations,
            &NoteSets::new(),
        )
        .unwrap();
        assert!(!lb_zktransfer::verify(deferred.proof(), deferred.inputs()).unwrap());
    }

    fn verified(
        operation: SDPWithdrawOp,
    ) -> SignedOperation<SDPWithdrawOp, Verified, StandardMode> {
        SignedOperation::<_, Unverified, StandardMode>::new(
            operation,
            <SDPWithdrawOp as ProvableOperation>::Proof::sample(),
        )
        .into_state_trusted()
    }

    // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
    // commitment.
    // #[test]
    // fn execute_schedules_the_withdrawal_after_the_snapshot_delay() {
    //     let operation = SDPWithdrawOp::sample();
    //     let declaration_id = operation.declaration_id;
    //     let locked_note_id = operation.service_note_id;
    //     let nonce = operation.nonce;
    //     let service_notes = locked_notes(&locked_note_id);
    //     let declarations = declarations(&operation, declaration(locked_note_id));
    //     let epoch = Epoch::from(4);
    //
    //     let (context, events) = verified(operation)
    //         .execute(SDPWithdrawExecutionContext {
    //             declarations,
    //             service_notes,
    //             epoch,
    //         })
    //         .expect("the declaration is registered");
    //
    //     let updated = context
    //         .declarations
    //         .get(&declaration_id)
    //         .expect("the declaration stays registered");
    //     assert_eq!(
    //         updated.withdraw_at,
    //         Some(epoch.strict_add(sdp::SNAPSHOT_FINALIZATION_DELAY))
    //     );
    //     assert_eq!(updated.nonce, nonce);
    //     assert!(
    //         context
    //             .service_notes
    //             .is_used_for_service(&locked_note_id, &ServiceType::BlendNetwork)
    //     );
    //     assert_eq!(events, []);
    // }

    #[test]
    fn execute_schedules_the_withdrawal_and_spends_the_service_note() {
        let operation = SDPWithdrawOp::sample();
        let declaration_id = operation.declaration_id;
        let declarations = declarations(&operation, declaration());
        let (context, events) = verified(operation)
            .execute(SDPWithdrawExecutionContext {
                declarations,
                note_sets: NoteSets::new(),
                epoch: Epoch::from(4),
            })
            .expect("the declaration is registered");

        let updated = context.declarations.get(&declaration_id).unwrap();
        assert_eq!(
            updated.withdraw_at,
            Some(Epoch::from(4).strict_add(sdp::SNAPSHOT_FINALIZATION_DELAY))
        );
        assert_eq!(updated.nonce, 29);
        assert_eq!(updated.withdraw_outputs, [NoteCm(Fr::from(46u64))].into());
        assert!(
            context
                .note_sets
                .get(SDP_SET)
                .unwrap()
                .contains_nullifier(&NoteNf(Fr::from(28u64)))
        );
        // The outputs wait for the removal of the declaration.
        assert_eq!(
            context.note_sets.get(LEDGER_SET).unwrap().cm_root(),
            NoteSet::empty().cm_root()
        );
        assert_eq!(events, []);
    }

    #[test]
    #[should_panic(expected = "The operation should have been validated")]
    fn execute_panics_on_a_declaration_the_ledger_does_not_hold() {
        let operation = SDPWithdrawOp::sample();
        // TODO(RFC-480): the service note is consumed by its nullifier against the root of its
        // commitment.
        // let service_notes = locked_notes(&operation.service_note_id);

        drop(verified(operation).execute(SDPWithdrawExecutionContext {
            declarations: Declarations::new_sync(),
            note_sets: NoteSets::new(),
            epoch: Epoch::from(4),
        }));
    }

    #[test]
    fn sdp_withdraw_op_execution_gas_does_not_scale_with_the_threshold() {
        for threshold in [0, 1, 3] {
            assert_eq!(
                SDPWithdrawOp::sample().execution_gas(&FixedThresholds(threshold)),
                Ok(Gas::new(678))
            );
        }
    }
}
