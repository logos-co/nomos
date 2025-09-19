use nomos_core::crypto::ZkHash;
use thiserror::Error;

use crate::crypto::proofs::{
    quota::{inputs::prove::PublicInputs, ProofOfQuota},
    selection::{inputs::VerifyInputs, ProofOfSelection},
};

pub mod quota;
pub mod selection;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid Proof of Quota: {0}.")]
    ProofOfQuota(#[from] quota::Error),
    #[error("Invalid Proof of Selection: {0}.")]
    ProofOfSelection(selection::Error),
}

/// Verifier that actually verifies the validity of Blend-related proofs.
#[derive(Clone)]
pub struct RealProofsVerifier;

impl crate::encap::ProofsVerifier for RealProofsVerifier {
    type Error = Error;

    fn new() -> Self {
        Self
    }

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error> {
        proof.verify(inputs).map_err(Error::ProofOfQuota)
    }

    fn verify_proof_of_selection(
        &self,
        proof: ProofOfSelection,
        inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        proof.verify(inputs).map_err(Error::ProofOfSelection)
    }
}
