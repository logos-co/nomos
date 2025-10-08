use nomos_blend_message::crypto::proofs::selection::inputs::VerifyInputs;
use test_log::test;

use crate::message_blend::provers::{
    ProofsGeneratorSettings,
    core_and_leader::{CoreAndLeaderProofsGenerator as _, RealCoreAndLeaderProofsGenerator},
    test_utils::{
        poq_public_inputs_from_session_public_inputs_and_signing_key, valid_proof_of_leader_inputs,
        valid_proof_of_quota_inputs,
    },
};

#[test(tokio::test)]
async fn proof_generation() {
    let core_quota = 10;
    let (public_inputs, core_private_inputs) = valid_proof_of_quota_inputs(core_quota);

    let mut core_and_leader_proofs_generator = RealCoreAndLeaderProofsGenerator::new(
        ProofsGeneratorSettings {
            local_node_index: None,
            membership_size: 1,
            public_inputs,
        },
        core_private_inputs,
    );

    for _ in 0..core_quota {
        let proof = core_and_leader_proofs_generator
            .get_next_core_proof()
            .await
            .unwrap();
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
    assert!(
        core_and_leader_proofs_generator
            .get_next_core_proof()
            .await
            .is_none()
    );

    let leadership_quota = 15;
    let (public_inputs, leadership_private_inputs) = valid_proof_of_leader_inputs(leadership_quota);

    core_and_leader_proofs_generator.override_settings(ProofsGeneratorSettings {
        local_node_index: None,
        membership_size: 1,
        public_inputs,
    });
    core_and_leader_proofs_generator.set_epoch_private(leadership_private_inputs);

    for _ in 0..leadership_quota {
        let proof = core_and_leader_proofs_generator
            .get_next_leader_proof()
            .await
            .unwrap();
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
