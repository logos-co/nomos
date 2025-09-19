use core::convert::Infallible;

use async_trait::async_trait;
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            quota::{inputs::prove::PublicInputs, ProofOfQuota},
            selection::{inputs::VerifyInputs, ProofOfSelection},
        },
    },
    encap::ProofsVerifier,
};
use nomos_blend_scheduling::message_blend::{BlendProof, ProofsGenerator, SessionInfo};
use nomos_core::crypto::ZkHash;

#[derive(Debug, Clone)]
pub struct MockProofsVerifier;

impl ProofsVerifier for MockProofsVerifier {
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

pub struct MockProofsGenerator;

#[async_trait]
impl ProofsGenerator for MockProofsGenerator {
    fn new(_session_info: SessionInfo) -> Self {
        Self
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendProof> {
        Some(mock_blend_proof())
    }

    async fn get_next_leadership_proof(&mut self) -> Option<BlendProof> {
        Some(mock_blend_proof())
    }
}

fn mock_blend_proof() -> BlendProof {
    BlendProof {
        proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
        proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
        ephemeral_signing_key: Ed25519PrivateKey::generate(),
    }
}
