use nomos_blend_message::crypto::{
    keys::Ed25519PublicKey,
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{
            fixtures::{valid_proof_of_core_quota_inputs, valid_proof_of_leadership_quota_inputs},
            inputs::prove::{
                PublicInputs as PoQPublicInputs,
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
            },
        },
        selection::inputs::VerifyInputs,
    },
};

use crate::message_blend::provers::{
    ProofsGeneratorSettings,
    core::{CoreProofsGenerator as _, RealCoreProofsGenerator},
    leader::{LeaderProofsGenerator as _, RealLeaderProofsGenerator},
};

const fn poq_public_inputs_from_session_public_inputs_and_signing_key(
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        signing_key,
    ): (PoQVerificationInputsMinusSigningKey, Ed25519PublicKey),
) -> PoQPublicInputs {
    PoQPublicInputs {
        signing_key,
        core,
        leader,
        session,
    }
}

fn valid_proof_of_quota_inputs(
    core_quota: u64,
) -> (PoQVerificationInputsMinusSigningKey, ProofOfCoreQuotaInputs) {
    let (
        PoQPublicInputs {
            core,
            leader,
            session,
            ..
        },
        private_inputs,
    ) = valid_proof_of_core_quota_inputs([0; _].try_into().unwrap(), core_quota);
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        private_inputs,
    )
}

fn valid_proof_of_leader_inputs(
    leader_quota: u64,
) -> (
    PoQVerificationInputsMinusSigningKey,
    ProofOfLeadershipQuotaInputs,
) {
    let (
        PoQPublicInputs {
            core,
            leader,
            session,
            ..
        },
        private_inputs,
    ) = valid_proof_of_leadership_quota_inputs([0; _].try_into().unwrap(), leader_quota);
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        private_inputs,
    )
}

#[tokio::test]
async fn real_core_proof_generation() {
    let core_quota = 10;
    let (public_inputs, private_inputs) = valid_proof_of_quota_inputs(core_quota);

    let mut core_proofs_generator = RealCoreProofsGenerator::new(
        ProofsGeneratorSettings {
            local_node_index: None,
            membership_size: 1,
            public_inputs,
        },
        private_inputs,
    );

    for _ in 0..core_quota {
        let proof = core_proofs_generator.get_next_proof().await.unwrap();
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
    assert!(core_proofs_generator.get_next_proof().await.is_none());
}

#[tokio::test]
async fn real_leader_proof_generation() {
    let leadership_quota = 15;
    let (public_inputs, private_inputs) = valid_proof_of_leader_inputs(leadership_quota);

    let mut leader_proofs_generator = RealLeaderProofsGenerator::new(
        ProofsGeneratorSettings {
            local_node_index: None,
            membership_size: 1,
            public_inputs,
        },
        private_inputs,
    );

    for _ in 0..leadership_quota {
        let proof = leader_proofs_generator.get_next_proof().await;
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
}
