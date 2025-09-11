use nomos_core::crypto::ZkHash;

use crate::crypto::proofs::quota::{inputs::prove::PublicInputs, ProofOfQuota};

pub mod decapsulated;
pub mod encapsulated;
pub mod unwrapped;

#[cfg(test)]
mod tests;

/// A set of methods to call on a Blend message to verify its proofs.
pub trait ProofsVerifier {
    type Error;

    /// Proof of Quota verification logic.
    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error>;
}
