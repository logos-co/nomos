use nomos_blend_message::crypto::proofs::selection::inputs::VerifyInputs;
use test_log::test;

use crate::message_blend::provers::{
    ProofsGeneratorSettings,
    core::{CoreProofsGenerator as _, RealCoreProofsGenerator},
    test_utils::{
        poq_public_inputs_from_session_public_inputs_and_signing_key, valid_proof_of_quota_inputs,
    },
};

#[test(tokio::test)]
async fn proof_generation() {
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

#[test(tokio::test)]
async fn epoch_rotation() {
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

    // Request all but the last proof, before rotating epoch (with the same public
    // data because proofs use hard-coded fixtures).
    for _ in 0..(core_quota - 1) {
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
                expected_node_index: 0,
                key_nullifier,
                total_membership_size: 1,
            })
            .unwrap();
    }

    let old_proof_generation_task_handle = core_proofs_generator
        .rotate_epoch_and_return_old_task(public_inputs.leader)
        .unwrap();

    // Old task should abort.
    old_proof_generation_task_handle.await.unwrap();

    // Generate and verify last proof.
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
            expected_node_index: 0,
            key_nullifier,
            total_membership_size: 1,
        })
        .unwrap();

    // Next proof should be `None` since we ran out of core quota.
    assert!(core_proofs_generator.get_next_proof().await.is_none());

    // Rotating epoch and requesting a new proof will also return `None.`

    core_proofs_generator.rotate_epoch(public_inputs.leader);
    assert!(core_proofs_generator.get_next_proof().await.is_none());
}
