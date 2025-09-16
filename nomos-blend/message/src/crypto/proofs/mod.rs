use nomos_core::crypto::ZkHash;
use thiserror::Error;

use crate::crypto::proofs::quota::{inputs::prove::PublicInputs, ProofOfQuota};

pub mod quota;
pub mod selection;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid Proof of Quota: {0}.")]
    ProofOfQuota(#[from] quota::Error),
}

/// Verifier that actually verifies the validity of Blend-related proofs.
pub struct ProofsVerifier;

impl crate::encap::ProofsVerifier for ProofsVerifier {
    type Error = Error;

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error> {
        proof.verify(inputs).map_err(Error::ProofOfQuota)
    }
}
