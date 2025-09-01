mod chain_inputs;
mod inputs;
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use std::error::Error;

pub use chain_inputs::{PolChainInputs, PolChainInputsData};
use groth16::{Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::PolWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PolWalletInputs, PolWalletInputsData};
pub use witness::Witness;

use crate::{
    chain_inputs::PolChainInputsJson,
    inputs::{PolVerifierInput, PolVerifierInputJson},
    proving_key::POL_PROVING_KEY_PATH,
};

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
pub fn prove(inputs: &PolWitnessInputs) -> Result<(PoLProof, PolVerifierInput), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, verifier_inputs) =
        circuits_prover::prover_from_contents(*POL_PROVING_KEY_PATH, witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let verifier_inputs: PolVerifierInputJson =
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
pub fn verify(proof: &PoLProof, public_inputs: &PolVerifierInput) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use groth16::Fr;
    use num_bigint::BigUint;
    use num_traits::Zero;

    use super::*;

    #[test]
    fn test_full_flow() {
        // let public_data = PolChainInputsData {
        //     entropy_contribution: BigUint::from_str(),
        //     slot_number: 511,
        //     epoch_nonce: 651,
        //     total_stake: 5000,
        //     aged_root: BigUint::from().into(),
        //     latest_root: BigUint::from().into(),
        //     leader_pk: (BigUint::from().into(), BigUint::from().into()),
        // };
        // let private_data = PolWalletInputsData {
        //     secret_key: Fr::zero(),
        //     note_value: 0,
        //     transaction_hash: [],
        //     output_number: 0,
        //     aged_proof: [],
        //     aged_path: vec![],
        //     aged_selector: vec![],
        //     latest_proof: [],
        //     latest_path: vec![],
        //     latest_selector: vec![],
        //     slot_secret: [],
        //     secrets_root: [],
        //     starting_slot: 0,
        // };
    }
}
