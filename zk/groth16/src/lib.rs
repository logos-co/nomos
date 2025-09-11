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

use std::error::Error;

pub use ark_bn254::{Bn254, Fr};
pub use ark_ff::Field;
use ark_ff::{BigInteger as _, PrimeField};
use num_bigint::BigUint;
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

pub type FrBytes = [u8; 32];

#[must_use]
pub fn fr_to_bytes(fr: &Fr) -> FrBytes {
    (*fr)
        .into_bigint()
        .to_bytes_le()
        .try_into()
        .expect("Bn254 Fr to bytes should fit in 32 bytes")
}

#[derive(Debug, thiserror::Error)]
#[error("Parsed bytes are bigger than the modulus, got {parsed_bytes}, expected {modulus}")]
pub struct FrFromBytesError {
    pub parsed_bytes: String,
    pub modulus: String,
}

pub fn fr_from_bytes(fr: &FrBytes) -> Result<Fr, impl Error> {
    let n = BigUint::from_bytes_le(fr);
    if n > <Fr as PrimeField>::MODULUS.into() {
        return Err(FrFromBytesError {
            parsed_bytes: n.to_string(),
            modulus: <Fr as PrimeField>::MODULUS.to_string(),
        });
    }
    Ok(n.into())
}

#[cfg(test)]
mod tests {
    use ark_bn254::Fr;
    use num_bigint::BigUint;

    use crate::{fr_from_bytes, fr_to_bytes};

    #[test]
    fn fr_to_from_bytes() {
        let value: Fr = BigUint::from(1_234_567_890_123_456_789u64).into();
        let bytes = fr_to_bytes(&value);
        let value2 = fr_from_bytes(&bytes).unwrap();
        assert_eq!(value, value2);
    }
}
