mod inputs;
mod private_inputs;
mod proving_key;
mod public_inputs;
mod verification_key;
mod witness;

use std::error::Error;

pub use inputs::PolInputs;
pub use private_inputs::{PolPrivateInputs, PolPrivateInputsData};
pub use public_inputs::{PolPublicInputs, PolPublicInputsData};
pub use verification_key::PolVerifyingKey;
pub type PoLProof = groth16::Groth16Proof;

pub fn verify(proof: &PoLProof, public_inputs: &PolPublicInputs) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), proof, &inputs)
}
