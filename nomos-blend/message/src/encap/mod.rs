use nomos_core::crypto::ZkHash;

use crate::crypto::proofs::{
    quota::{ProofOfQuota, inputs::prove::PublicInputs},
    selection::{ProofOfSelection, inputs::VerifyInputs},
};

pub mod decapsulated;
pub mod encapsulated;
pub mod validated;

#[cfg(test)]
mod tests;

/// A set of methods to call on a Blend message to verify its proofs.
pub trait ProofsVerifier {
    type Error;

    fn new() -> Self;

    /// Proof of Quota verification logic.
    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error>;

    /// Proof of Selection verification logic.
    fn verify_proof_of_selection(
        &self,
        proof: ProofOfSelection,
        inputs: &VerifyInputs,
    ) -> Result<(), Self::Error>;
}
