mod curve;
mod proof;
mod protocol;
mod verification_key;
mod verifier;

use ark_bn254::Bn254;
pub use verifier::groth16_verify;

pub type Groth16Proof = proof::Proof<Bn254>;
pub type Groth16VerificationKey = verification_key::VerificationKey<Bn254>;
