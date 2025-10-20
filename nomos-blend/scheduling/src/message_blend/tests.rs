use nomos_blend_message::crypto::{
    keys::Ed25519PublicKey,
    proofs::{
        quota::{
            fixtures::{valid_proof_of_core_quota_inputs, valid_proof_of_leadership_quota_inputs},
            inputs::prove::{
                PrivateInputs as PoQPrivateInputs, PublicInputs as PoQPublicInputs,
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs, ProofType},
            },
        },
        selection::inputs::VerifyInputs,
    },
};

use crate::message_blend::{
    PrivateInputs, ProofsGenerator as _, PublicInputs, RealProofsGenerator, SessionInfo,
};

const fn poq_public_inputs_from_session_public_inputs_and_signing_key(
    (
        PublicInputs {
            core_quota,
            core_root,
            leader_quota,
            pol_epoch_nonce,
            pol_ledger_aged,
            session,
            total_stake,
        },
        signing_key,
    ): (PublicInputs, Ed25519PublicKey),
) -> PoQPublicInputs {
    PoQPublicInputs {
        core_quota,
        core_root,
        leader_quota,
        pol_epoch_nonce,
        pol_ledger_aged,
        session,
        total_stake,
        signing_key,
    }
}

// We combine the relevant fields for each of the valid proof fixtures, to
// return inputs that can be used to generate either PoQ type.
fn valid_proof_of_quota_inputs(
    core_quota: u64,
    leader_quota: u64,
) -> (PublicInputs, PrivateInputs) {
    // We use key index of `0` for both proofs, but we will replace it inside the
    // proof generator, since it's not part of the fixtures.
    let (
        PoQPublicInputs { core_root, .. },
        PoQPrivateInputs {
            proof_type: core_proof_type,
            ..
        },
    ) = valid_proof_of_core_quota_inputs([0; _].try_into().unwrap(), core_quota, 0);
    let (
        PoQPublicInputs {
            pol_epoch_nonce,
            pol_ledger_aged,
            session,
            total_stake,
            ..
        },
        PoQPrivateInputs {
            proof_type: leadership_proof_type,
            ..
        },
    ) = valid_proof_of_leadership_quota_inputs([0; _].try_into().unwrap(), leader_quota, 0);

    let ProofType::CoreQuota(proof_of_core_quota_private_inputs) = core_proof_type else {
        panic!("Core proof private inputs of wrong type.");
    };

    let ProofType::LeadershipQuota(proof_of_leadership_quota_private_inputs) =
        leadership_proof_type
    else {
        panic!("Leadership proof private inputs of wrong type.");
    };

    let ProofOfCoreQuotaInputs {
        core_path_and_selectors,
        core_sk,
    } = *proof_of_core_quota_private_inputs;
    let ProofOfLeadershipQuotaInputs {
        aged_path_and_selectors,
        note_value,
        output_number,
        pol_secret_key,
        slot,
        slot_secret,
        slot_secret_path,
        starting_slot,
        transaction_hash,
    } = *proof_of_leadership_quota_private_inputs;

    let public_inputs = PublicInputs {
        session,
        core_root,
        pol_ledger_aged,
        pol_epoch_nonce,
        core_quota,
        leader_quota,
        total_stake,
    };
    let private_inputs = PrivateInputs {
        core_sk,
        core_path_and_selectors,
        slot,
        note_value,
        transaction_hash,
        output_number,
        aged_path_and_selectors,
        slot_secret,
        slot_secret_path,
        starting_slot,
        pol_secret_key,
    };

    (public_inputs, private_inputs)
}

#[tokio::test]
async fn real_proof_generation() {
    let core_quota = 10;
    let leadership_quota = 15;
    let (public_inputs, private_inputs) = valid_proof_of_quota_inputs(core_quota, leadership_quota);

    let mut proofs_generator = RealProofsGenerator::new(SessionInfo {
        local_node_index: None,
        membership_size: 1,
        private_inputs,
        public_inputs,
    });

    for _ in 0..core_quota {
        let proof = proofs_generator.get_next_core_proof().await.unwrap();
        let key_nullifier = proof
            .proof_of_quota
            .verify(
                &poq_public_inputs_from_session_public_inputs_and_signing_key((
                    public_inputs,
                    proof.ephemeral_signing_key.public_key(),
                )),
            )
            .unwrap();
        proof
            .proof_of_selection
            .verify(&VerifyInputs {
                // Membership of 1 -> only a single index can be included
                expected_node_index: 0,
                key_nullifier,
                total_membership_size: 1,
            })
            .unwrap();
    }

    // Next proof should be `None` since we ran out of core quota.
    assert!(proofs_generator.get_next_core_proof().await.is_none());

    for _ in 0..leadership_quota {
        let proof = proofs_generator.get_next_leadership_proof().await.unwrap();
        let key_nullifier = proof
            .proof_of_quota
            .verify(
                &poq_public_inputs_from_session_public_inputs_and_signing_key((
                    public_inputs,
                    proof.ephemeral_signing_key.public_key(),
                )),
            )
            .unwrap();
        proof
            .proof_of_selection
            .verify(&VerifyInputs {
                // Membership of 1 -> only a single index can be included
                expected_node_index: 0,
                key_nullifier,
                total_membership_size: 1,
            })
            .unwrap();
    }

    // Next proof should be `None` since we ran out of leadership quota.
    assert!(proofs_generator.get_next_leadership_proof().await.is_none());
}
