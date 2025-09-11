use nomos_core::crypto::ZkHash;

use crate::crypto::proofs::quota::{inputs::prove::PublicInputs, ProofOfQuota};

pub mod decapsulated;
pub mod encapsulated;
pub mod unwrapped;

#[cfg(test)]
mod tests;

pub trait ProofsVerifier {
    type Error;

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error>;
}
