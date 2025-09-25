use groth16::Field as _;
use nomos_core::crypto::ZkHash;

use crate::crypto::proofs::{
    ZkHashExt as _,
    selection::{Error, ProofOfSelection, inputs::VerifyInputs},
};

#[test]
fn success_on_valid_proof() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let expected_key_nullifier = [ZkHash::ZERO].hash();
    posel
        .verify(&VerifyInputs {
            expected_node_index: 0,
            total_membership_size: 1,
            key_nullifier: expected_key_nullifier,
        })
        .unwrap();
}

#[test]
fn fail_on_invalid_nullifier() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let Err(Error::KeyNullifierMismatch { expected, provided }) = posel.verify(&VerifyInputs {
        expected_node_index: 0,
        total_membership_size: 1,
        key_nullifier: ZkHash::ONE,
    }) else {
        panic!("posel.verify should fail.");
    };
    assert_eq!(expected, [ZkHash::ZERO].hash());
    assert_eq!(provided, ZkHash::ONE);
}

#[test]
fn fail_on_invalid_index() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let expected_index = posel.expected_index(2).unwrap();
    let Err(Error::IndexMismatch { expected, provided }) = posel.verify(&VerifyInputs {
        // We expect the opposite index.
        expected_node_index: 1 - expected_index as u64,
        total_membership_size: 2,
        key_nullifier: [ZkHash::ZERO].hash(),
    }) else {
        panic!("posel.verify should fail.");
    };
    assert_eq!(expected as usize, expected_index);
    assert_eq!(provided, 1 - expected_index as u64);
}
