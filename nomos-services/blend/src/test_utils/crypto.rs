use core::convert::Infallible;

use async_trait::async_trait;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey},
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{
                ProofOfQuota,
                inputs::prove::{
                    private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
                    public::LeaderInputs,
                },
            },
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
    },
    encap::ProofsVerifier,
};
use nomos_blend_scheduling::message_blend::provers::{
    BlendLayerProof, ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator,
};
use nomos_core::crypto::ZkHash;

pub struct MockCoreAndLeaderProofsGenerator;

#[async_trait]
impl CoreAndLeaderProofsGenerator for MockCoreAndLeaderProofsGenerator {
    fn new(_settings: ProofsGeneratorSettings, _private_inputs: ProofOfCoreQuotaInputs) -> Self {
        Self
    }

    fn rotate_epoch(&mut self, _new_epoch_public: LeaderInputs) {}

    fn set_epoch_private(&mut self, _new_epoch_private: ProofOfLeadershipQuotaInputs) {}

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(mock_blend_proof())
    }

    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
        Some(mock_blend_proof())
    }
}

#[derive(Debug, Clone)]
pub struct MockProofsVerifier;

impl ProofsVerifier for MockProofsVerifier {
    type Error = Infallible;

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
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

pub fn mock_blend_proof() -> BlendLayerProof {
    BlendLayerProof {
        proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
        proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
        ephemeral_signing_key: Ed25519PrivateKey::generate(),
    }
}
