#[cfg(feature = "deser")]
mod curve;
#[cfg(feature = "deser")]
mod from_json_error;
mod proof;
#[cfg(feature = "deser")]
mod protocol;
mod public_input;

#[cfg(feature = "deser")]
pub mod serde;
pub(crate) mod utils;
mod verification_key;
mod verifier;

pub use ark_bn254::{Bn254, Fr};
pub use ark_ff::Field;
pub use verifier::groth16_verify;
const BN254_G1_COMPRESSED_SIZE: usize = 32;
const BN254_G2_COMPRESSED_SIZE: usize = 64;
pub type Groth16Proof = proof::Proof<Bn254>;
pub type CompressedGroth16Proof =
    proof::CompressedProof<BN254_G1_COMPRESSED_SIZE, BN254_G2_COMPRESSED_SIZE, Bn254>;
#[cfg(feature = "deser")]
pub type Groth16ProofJsonDeser = proof::ProofJsonDeser;
pub type Groth16VerificationKey = verification_key::VerificationKey<Bn254>;
pub type Groth16PreparedVerificationKey = verification_key::PreparedVerificationKey<Bn254>;
#[cfg(feature = "deser")]
pub type Groth16VerificationKeyJsonDeser = verification_key::VerificationKeyJsonDeser;
pub type Groth16Input = public_input::Input<Bn254>;
#[cfg(feature = "deser")]
pub type Groth16InputDeser = public_input::InputDeser;
