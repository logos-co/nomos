use core::time::Duration;

use nomos_blend_message::crypto::proofs::selection::inputs::VerifyInputs;
use test_log::test;
use tokio::time::timeout;

use crate::message_blend::provers::{
    ProofsGeneratorSettings,
    leader::{LeaderProofsGenerator as _, RealLeaderProofsGenerator},
    test_utils::{
        poq_public_inputs_from_session_public_inputs_and_signing_key, valid_proof_of_leader_inputs,
    },
};

#[test(tokio::test)]
async fn proof_generation() {
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

    // Next proof should still return `Some` since leadership proofs do not have a
    // maximum cap.
    timeout(
        Duration::from_secs(3),
        leader_proofs_generator.get_next_proof(),
    )
    .await
    .unwrap();
}

#[test(tokio::test)]
async fn epoch_rotation() {
    let leadership_quota = 15;
    let (public_inputs, private_inputs) = valid_proof_of_leader_inputs(leadership_quota);

    let mut leader_proofs_generator = RealLeaderProofsGenerator::new(
        ProofsGeneratorSettings {
            local_node_index: None,
            membership_size: 1,
            public_inputs,
        },
        private_inputs.clone(),
    );

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
            expected_node_index: 0,
            key_nullifier,
            total_membership_size: 1,
        })
        .unwrap();

    let old_proof_generation_task_handle = leader_proofs_generator
        .rotate_epoch_and_return_old_task(public_inputs.leader, private_inputs)
        .unwrap();

    // Old task should abort.
    old_proof_generation_task_handle.await.unwrap();

    // Generate and verify new proof.
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
            expected_node_index: 0,
            key_nullifier,
            total_membership_size: 1,
        })
        .unwrap();
}
