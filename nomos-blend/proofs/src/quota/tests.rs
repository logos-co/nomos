use const_hex::FromHex as _;
use groth16::fr_from_bytes_unchecked;
use nomos_blend_crypto::keys::{ED25519_PUBLIC_KEY_SIZE, Ed25519PublicKey};

use crate::{
    quota::{
        DOMAIN_SEPARATION_TAG_FR, VerifiedProofOfQuota,
        fixtures::{valid_proof_of_core_quota_inputs, valid_proof_of_leadership_quota_inputs},
        inputs::prove::PrivateInputs,
    },
    selection::derive_key_nullifier_from_secret_selection_randomness,
};

#[test]
fn secret_selection_randomness_dst_encoding() {
    // Blend spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#25e261aa09df802d87edfc54d1d60b80>
    assert_eq!(
        *DOMAIN_SEPARATION_TAG_FR,
        fr_from_bytes_unchecked(
            &<[u8; 23]>::from_hex("0x53454c454354494f4e5f52414e444f4d4e4553535f5631").unwrap()
        ),
    );
}

#[test]
fn valid_proof_of_core_quota() {
    let (public_inputs, private_inputs) = valid_proof_of_core_quota_inputs(
        Ed25519PublicKey::from_bytes(&[0; ED25519_PUBLIC_KEY_SIZE]).unwrap(),
        1,
    );

    let (proof, secret_selection_randomness) = VerifiedProofOfQuota::new(
        &public_inputs,
        PrivateInputs::new_proof_of_core_quota_inputs(0, private_inputs),
    )
    .unwrap();

    let verified_proof_of_quota = proof.into_inner().verify(&public_inputs).unwrap();
    assert_eq!(
        derive_key_nullifier_from_secret_selection_randomness(secret_selection_randomness),
        verified_proof_of_quota.key_nullifier()
    );
}

// We test that our assumption that two PoQs with the exact same public and
// private inputs but different ephemeral key still produce the same nullifier.
#[test]
fn same_key_nullifier_for_different_public_keys() {
    let key_1: Ed25519PublicKey =
        Ed25519PublicKey::from_bytes(&[200; ED25519_PUBLIC_KEY_SIZE]).unwrap();
    let key_2: Ed25519PublicKey =
        Ed25519PublicKey::from_bytes(&[250; ED25519_PUBLIC_KEY_SIZE]).unwrap();

    let (public_inputs_key_1, private_inputs_key_1) = valid_proof_of_core_quota_inputs(key_1, 1);
    let (public_inputs_key_2, private_inputs_key_2) = valid_proof_of_core_quota_inputs(key_2, 1);

    let (proof_key_1, _) = VerifiedProofOfQuota::new(
        &public_inputs_key_1,
        PrivateInputs::new_proof_of_core_quota_inputs(0, private_inputs_key_1),
    )
    .unwrap();
    let verified_proof_of_quota_1 = proof_key_1
        .into_inner()
        .verify(&public_inputs_key_1)
        .unwrap();
    let (proof_key_2, _) = VerifiedProofOfQuota::new(
        &public_inputs_key_2,
        PrivateInputs::new_proof_of_core_quota_inputs(0, private_inputs_key_2),
    )
    .unwrap();
    let verified_proof_of_quota_2 = proof_key_2
        .into_inner()
        .verify(&public_inputs_key_2)
        .unwrap();

    assert_eq!(
        verified_proof_of_quota_1.key_nullifier(),
        verified_proof_of_quota_2.key_nullifier()
    );
}

#[test]
fn valid_proof_of_leadership_quota() {
    let (public_inputs, private_inputs) = valid_proof_of_leadership_quota_inputs(
        Ed25519PublicKey::from_bytes(&[0; ED25519_PUBLIC_KEY_SIZE]).unwrap(),
        1,
    );

    let (proof, secret_selection_randomness) = VerifiedProofOfQuota::new(
        &public_inputs,
        PrivateInputs::new_proof_of_leadership_quota_inputs(0, private_inputs),
    )
    .unwrap();

    let verified_proof_of_quota = proof.into_inner().verify(&public_inputs).unwrap();
    assert_eq!(
        derive_key_nullifier_from_secret_selection_randomness(secret_selection_randomness),
        verified_proof_of_quota.key_nullifier()
    );
}
