use groth16::Fr;

use crate::crypto::proofs::quota::{inputs::prove::PublicInputs, ProofOfQuota};

pub mod quota;
pub mod selection;

pub enum Error {
    ProofOfQuota(quota::Error),
}

pub struct ProofsVerifier;

impl crate::encap::ProofsVerifier for ProofsVerifier {
    type Error = Error;
    type ProofOfQuotaVerificationOutput = Fr;

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        inputs: &PublicInputs,
    ) -> Result<Self::ProofOfQuotaVerificationOutput, Self::Error> {
        proof.verify(inputs).map_err(Error::ProofOfQuota)
    }
}
