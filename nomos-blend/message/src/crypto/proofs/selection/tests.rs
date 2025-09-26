use const_hex::FromHex as _;
use groth16::Field as _;
use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;

use crate::crypto::proofs::selection::{
    Error, KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG_FR, ProofOfSelection,
    derive_key_nullifier_from_secret_selection_randomness, inputs::VerifyInputs,
};

#[test]
fn secret_selection_randomness_to_key_nullifier_dst_encoding() {
    // Blend spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#26c261aa09df802f9dbcd0780dc7ac6e>
    assert_eq!(
        *KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG_FR,
        BigUint::from_bytes_be(
            &<[u8; 16]>::from_hex("0x31565f52454946494c4c554e5f59454b").unwrap()
        )
        .into()
    );
}

#[test]
fn success_on_valid_proof() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let expected_key_nullifier =
        derive_key_nullifier_from_secret_selection_randomness(ZkHash::ZERO);
    posel
        .verify(&VerifyInputs {
            expected_node_index: 0,
            total_membership_size: 1,
            key_nullifier: expected_key_nullifier,
        })
        .unwrap();
}

#[test]
fn failure_on_invalid_nullifier() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let Err(Error::KeyNullifierMismatch { expected, provided }) = posel.verify(&VerifyInputs {
        expected_node_index: 0,
        total_membership_size: 1,
        key_nullifier: ZkHash::ONE,
    }) else {
        panic!("`posel.verify` should fail.");
    };
    assert_eq!(
        expected,
        derive_key_nullifier_from_secret_selection_randomness(ZkHash::ZERO)
    );
    assert_eq!(provided, ZkHash::ONE);
}

#[test]
fn failure_on_invalid_index() {
    let posel = ProofOfSelection::new(ZkHash::ZERO);
    let expected_index = posel.expected_index(2).unwrap();
    let Err(Error::IndexMismatch { expected, provided }) = posel.verify(&VerifyInputs {
        // We expect the opposite index.
        expected_node_index: 1 - expected_index as u64,
        total_membership_size: 2,
        key_nullifier: derive_key_nullifier_from_secret_selection_randomness(ZkHash::ZERO),
    }) else {
        panic!("posel.verify should fail.");
    };
    assert_eq!(expected as usize, expected_index);
    assert_eq!(provided, 1 - expected_index as u64);
}
