use core::convert::Infallible;

use nomos_blend_message::{
    crypto::proofs::{
        quota::{inputs::prove::PublicInputs, ProofOfQuota},
        selection::{inputs::VerifyInputs, ProofOfSelection},
    },
    encap::{encapsulated::PoQVerificationInputMinusSigningKey, ProofsVerifier},
};
use nomos_core::crypto::ZkHash;

#[derive(Debug, Clone)]
pub struct NeverFailingProofsVerifier;

impl ProofsVerifier for NeverFailingProofsVerifier {
    type Error = Infallible;

    fn new() -> Self {
        Self
    }

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
