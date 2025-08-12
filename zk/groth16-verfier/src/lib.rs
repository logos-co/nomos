#[cfg(feature = "deser")]
mod curve;
#[cfg(feature = "deser")]
mod from_json_error;
mod proof;
#[cfg(feature = "deser")]
mod protocol;
mod public_input;
pub(crate) mod utils;
mod verification_key;
mod verifier;

use ark_bn254::Bn254;
pub use verifier::groth16_verify;

pub type Groth16Proof = proof::Proof<Bn254>;
#[cfg(feature = "deser")]
pub type Groth16ProofJsonDeser = proof::ProofJsonDeser;
pub type Groth16VerificationKey = verification_key::VerificationKey<Bn254>;
pub type Groth16PreparedVerificationKey = verification_key::PreparedVerificationKey<Bn254>;
#[cfg(feature = "deser")]
pub type Groth16VerificationKeyJsonDeser = verification_key::VerificationKeyJsonDeser;
pub type Groth16PublicInput = public_input::PublicInput<Bn254>;
#[cfg(feature = "deser")]
pub type Groth16PublicInputDeser = public_input::PublicInputDeser;
