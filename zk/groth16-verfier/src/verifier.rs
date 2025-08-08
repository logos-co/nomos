use std::error::Error;

use ark_ec::pairing::Pairing;
use ark_groth16::{Groth16, r1cs_to_qap::LibsnarkReduction};

use crate::{proof::Proof, verification_key::PreparedVerificationKey};

pub fn groth16_verify<E: Pairing>(
    vk: &PreparedVerificationKey<E>,
    public_inputs: &[E::ScalarField],
    proof: &Proof<E>,
) -> Result<bool, impl Error> {
    let proof: ark_groth16::Proof<E> = proof.into();
    Groth16::<E, LibsnarkReduction>::verify_proof(vk.as_ref(), &proof, public_inputs)
}
