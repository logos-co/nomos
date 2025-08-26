mod inputs;
mod private_inputs;
mod proving_key;
mod public_inputs;
mod verification_key;
mod witness;

use std::error::Error;

use groth16::{Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::PolInputs;
pub use private_inputs::{PolPrivateInputs, PolPrivateInputsData};
pub use public_inputs::{PolPublicInputs, PolPublicInputsData};
use thiserror::Error;
pub use witness::Witness;

use crate::{proving_key::POL_PROVING_KEY_PATH, public_inputs::PolPublicInputsJson};

pub type PoLProof = Groth16Proof;

#[derive(Debug, Error)]
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
/// - `inputs`: A reference to `PolInputs`, which contains the necessary data to
///   generate the witness and construct the proof.
///
/// # Returns
/// - `Ok((PoLProof, PolPublicInputs))`: On success, returns a tuple containing
///   the generated proof (`PoLProof`) and the corresponding public inputs
///   (`PolPublicInputs`).
/// - `Err(ProveError)`: On failure, returns an error of type `ProveError`,
///   which can occur due to I/O errors or JSON (de)serialization errors.
///
/// # Errors
/// - Returns a `ProveError::Io` if an I/O error occurs while generating the
///   witness or proving from contents.
/// - Returns a `ProveError::Json` if there is an error during JSON
///   serialization or deserialization.
pub fn prove(inputs: &PolInputs) -> Result<(PoLProof, PolPublicInputs), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, public_inputs) =
        circuits_prover::prover_from_contents(*POL_PROVING_KEY_PATH, witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let public_inputs: PolPublicInputsJson =
        serde_json::from_slice(&public_inputs).map_err(ProveError::Json)?;
    Ok((
        proof.try_into().map_err(ProveError::Groth16JsonProof)?,
        public_inputs
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
/// - `public_inputs`: A reference to `PolPublicInputs`, which contains the
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
pub fn verify(proof: &PoLProof, public_inputs: &PolPublicInputs) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_flow() {
        let public_data = PolPublicInputsData {
            entropy_contribution: [0; 32],
            slot_number: 10,
            epoch_nonce: 125,
            total_stake: 50,
            aged_root: [0; 32],
            latest_root: [0; 32],
            leader_pk: (
                (0..16).collect::<Vec<_>>().try_into().unwrap(),
                (0..16).rev().collect::<Vec<_>>().try_into().unwrap(),
            ),
        };
        let private_data = PolPrivateInputsData {
            secret_key: [],
            note_value: 0,
            transaction_hash: [],
            output_numer: 0,
            aged_proof: [],
            aged_path: vec![],
            aged_selector: vec![],
            latest_proof: [],
            latest_path: vec![],
            latest_selector: vec![],
            slot_secret: [],
            secrets_root: [],
            starting_slot: 0,
        };
    }
}
