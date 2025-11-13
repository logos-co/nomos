use core::{cell::Cell, convert::Infallible};

use async_trait::async_trait;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey},
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{
                ProofOfQuota,
                inputs::prove::{private::ProofOfLeadershipQuotaInputs, public::LeaderInputs},
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
impl<CorePoQGenerator> CoreAndLeaderProofsGenerator<CorePoQGenerator>
    for MockCoreAndLeaderProofsGenerator
{
    fn new(
        _settings: ProofsGeneratorSettings,
        _core_proof_of_quota_generator: CorePoQGenerator,
    ) -> Self {
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

thread_local! {
    /// Static value used by the `StaticFetchVerifier` below to count after how many
    /// `Ok`s it should return `Err`s when verifying encapsulated message layers.
    ///
    /// This value refers to proof of selections, since when decapsulating a message, we already assume the `PoQ`
    /// in the public header was correct, so we use `PoSel` to control the number of `Ok`s before failing at the given level.
    static REMAINING_VALID_LAYERS: Cell<u64> = const { Cell::new(0) };
}

#[derive(Debug, Clone)]
pub struct StaticFetchVerifier;

impl StaticFetchVerifier {
    pub fn set_remaining_valid_poq_proofs(remaining_valid_proofs: u64) {
        REMAINING_VALID_LAYERS.with(|val| val.set(remaining_valid_proofs));
    }
}

impl ProofsVerifier for StaticFetchVerifier {
    type Error = ();

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
        REMAINING_VALID_LAYERS.with(|val| {
            let remaining = val.get();
            if remaining > 0 {
                val.set(remaining - 1);
                Ok(())
            } else {
                Err(())
            }
        })
    }
}

pub fn mock_blend_proof() -> BlendLayerProof {
    BlendLayerProof {
        proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
        proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
        ephemeral_signing_key: Ed25519PrivateKey::generate(),
    }
}
