use ark_bn254::Fr;
use ark_ec::pairing::Pairing;

use crate::{proof::Proof, verification_key::VerificationKey};

pub fn groth16_verify<E: Pairing>(
    vk: &VerificationKey<E>,
    public_inputs: &[E::G1],
    proof: Proof<E>,
) -> bool {
    assert_eq!(public_inputs.len(), vk.public_size());
    let VerificationKey {
        public_size,
        alpha_1,
        beta_2,
        gamma_2,
        delta_2,
        alpha_beta_12,
        ic,
    } = vk;
    let ic0 = ic[0];
    let w = Fr::from(public_size);

    true
}
