use nomos_core::crypto::{ZkHash, ZkHasher};
use thiserror::Error;

use crate::{
    crypto::proofs::{
        quota::{ProofOfQuota, inputs::prove::PublicInputs},
        selection::{ProofOfSelection, inputs::VerifyInputs},
    },
    encap::ProofsVerifier,
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

impl ProofsVerifier for RealProofsVerifier {
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

trait ZkHashExt {
    fn hash(&self) -> ZkHash;
}

impl<T> ZkHashExt for T
where
    T: AsRef<[ZkHash]>,
{
    fn hash(&self) -> ZkHash {
        let mut hasher = ZkHasher::new();
        hasher.update(self.as_ref());
        hasher.finalize()
    }
}
