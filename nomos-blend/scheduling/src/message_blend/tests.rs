use core::str::FromStr;

use nomos_blend_message::crypto::{
    keys::Ed25519PublicKey,
    proofs::quota::{
        fixtures::valid_proof_of_core_quota_inputs, inputs::prove::PublicInputs as PoQPublicInputs,
    },
};
use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;

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

#[tokio::test]
async fn real_proof_generation() {
    // TODO: Resume from here
    let (private_inputs, public_inputs) =
        valid_proof_of_core_quota_inputs([0; _].try_into().unwrap(), 1, 0);
    let core_quota = public_inputs.core_quota;
    let leadership_quota = public_inputs.leader_quota;
    let mut proofs_generator = RealProofsGenerator::new(SessionInfo {
        local_node_index: None,
        membership_size: 1,
        private_inputs,
        public_inputs,
    });

    for _ in 0..core_quota {
        println!("AAAAA");
        let proof = proofs_generator.get_next_core_proof().await.unwrap();
        proof
            .proof_of_quota
            .verify(
                &poq_public_inputs_from_session_public_inputs_and_signing_key((
                    public_inputs,
                    proof.ephemeral_signing_key.public_key(),
                )),
            )
            .unwrap();
    }

    // Next proof should be `None` since we ran out of core quota.
    assert!(proofs_generator.get_next_core_proof().await.is_none());

    for _ in 0..leadership_quota {
        let proof = proofs_generator.get_next_leadership_proof().await.unwrap();
        // proof
        //     .proof_of_quota
        //     .verify(
        //         &poq_public_inputs_from_session_public_inputs_and_signing_key((
        //             public_inputs,
        //             proof.ephemeral_signing_key.public_key(),
        //         )),
        //     )
        //     .unwrap();
    }

    // Next proof should be `None` since we ran out of leadership quota.
    assert!(proofs_generator.get_next_leadership_proof().await.is_none());
}
