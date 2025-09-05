mod inputs;
mod private;
mod proving_key;
mod public;
mod verification_key;
mod witness;

use std::error::Error;

use groth16::{Fr, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::ZkSignWitnessInputs;
pub use public::ZkSignVerifierInputs;

use crate::{proving_key::ZKSIGN_PROVING_KEY_PATH, public::ZkSignVerifierInputsJson};

pub type ZkSignProof = Groth16Proof;

#[derive(Debug, thiserror::Error)]
pub enum ProveError {
    #[error(transparent)]
    Io(std::io::Error),
    #[error(transparent)]
    Json(serde_json::Error),
    #[error("Error parsing Groth16 input: {0:?}")]
    Groth16JsonInput(<Groth16Input as TryFrom<Groth16InputDeser>>::Error),
    #[error(transparent)]
    Groth16JsonProof(<Groth16Proof as TryFrom<Groth16ProofJsonDeser>>::Error),
}

///
/// This function generates a proof for the given set of inputs.
///
/// # Arguments
/// - `inputs`: A reference to `PolWitnessInputs`, which contains the necessary
///   data to generate the witness and construct the proof.
///
/// # Returns
/// - `Ok((PoLProof, PolVerifierInput))`: On success, returns a tuple containing
///   the generated proof (`PoLProof`) and the corresponding public inputs
///   (`PolVerifierInput`).
/// - `Err(ProveError)`: On failure, returns an error of type `ProveError`,
///   which can occur due to I/O errors or JSON (de)serialization errors.
///
/// # Errors
/// - Returns a `ProveError::Io` if an I/O error occurs while generating the
///   witness or proving from contents.
/// - Returns a `ProveError::Json` if there is an error during JSON
///   serialization or deserialization.
pub fn prove(
    inputs: &ZkSignWitnessInputs,
) -> Result<(ZkSignProof, ZkSignVerifierInputs), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, verifier_inputs) =
        circuits_prover::prover_from_contents(ZKSIGN_PROVING_KEY_PATH.as_path(), witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let verifier_inputs: ZkSignVerifierInputsJson =
        serde_json::from_slice(&verifier_inputs).map_err(ProveError::Json)?;
    Ok((
        proof.try_into().map_err(ProveError::Groth16JsonProof)?,
        verifier_inputs
            .try_into()
            .map_err(ProveError::Groth16JsonInput)?,
    ))
}

///
/// This function verifies a proof against a set of public inputs.
///
/// # Arguments
///
/// - `proof`: A reference to the proof (`PoLProof`) that needs verification.
/// - `public_inputs`: A reference to `PolVerifierInput`, which contains the
///   public inputs against which the proof is verified.
///
/// # Returns
///
/// - `Ok(true)`: If the proof is successfully verified against the public
///   inputs.
/// - `Ok(false)`: If the proof is invalid when compared with the public inputs.
/// - `Err`: If an error occurs during the verification process.
///
/// # Errors
///
/// - Returns an error if there is an issue with the verification key or the
///   underlying verification process fails.
pub fn verify(
    proof: &ZkSignProof,
    public_inputs: &ZkSignVerifierInputs,
) -> Result<bool, impl Error> {
    let inputs: [Fr; _] = public_inputs.public_keys.map(Groth16Input::into_inner);
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;
    use poseidon2::{Digest as _, Poseidon2Bn254Hasher};
    use rand::RngCore as _;

    use super::*;
    use crate::private::ZkSignPrivateKeysData;

    #[test]
    fn test_full_flow() {
        let mut rng = rand::thread_rng();
        let sks: [Fr; 32] = std::iter::repeat_with(|| BigUint::from(rng.next_u64()).into())
            .take(32)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let sks: ZkSignPrivateKeysData = sks.into();
        let msg_hash = Poseidon2Bn254Hasher::digest(&[BigUint::from_bytes_le(b"foo_bar").into()]);
        let input = ZkSignWitnessInputs::from_witness_data_and_message_hash(sks, msg_hash);
        let (proof, verifier_inputs) = prove(&input).unwrap();
        assert!(verify(&proof, &verifier_inputs).unwrap());
    }
}
