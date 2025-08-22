mod inputs;
mod private_inputs;
mod proving_key;
mod public_inputs;
mod verification_key;

pub use inputs::PolInputs;
pub use private_inputs::{PolPrivateInputs, PolPrivateInputsData};
pub use proving_key::PolProvingKey;
pub use public_inputs::{PolPublicInputs, PolPublicInputsData};
pub use verification_key::PolVerifyingKey;
pub type PoLProof = groth16::Groth16Proof;
