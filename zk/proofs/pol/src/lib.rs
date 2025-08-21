mod proving_key;
mod public_inputs;
mod verification_key;

pub use proving_key::PolProvingKey;
pub use public_inputs::PolPublicInputs;
pub use verification_key::PolVerifyingKey;
pub type PoLProof = groth16::Groth16Proof;
