use core::convert::Infallible;

use async_trait::async_trait;
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PublicKey,
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
use nomos_core::crypto::ZkHash;

use crate::message_blend::provers::{
    BlendLayerProof, ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator,
    leader::LeaderProofsGenerator,
};

pub struct TestEpochChangeLeaderProofsGenerator(
    pub ProofsGeneratorSettings,
    pub ProofOfLeadershipQuotaInputs,
);

#[async_trait]
impl LeaderProofsGenerator for TestEpochChangeLeaderProofsGenerator {
    fn new(
        settings: ProofsGeneratorSettings,
        private_inputs: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        Self(settings, private_inputs)
    }

    fn rotate_epoch(
        &mut self,
        new_epoch_public: LeaderInputs,
        new_private_inputs: ProofOfLeadershipQuotaInputs,
    ) {
        self.0.public_inputs.leader = new_epoch_public;
        self.1 = new_private_inputs;
    }

    async fn get_next_proof(&mut self) -> BlendLayerProof {
        BlendLayerProof {
            proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
            proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
            ephemeral_signing_key: [0; _].into(),
        }
    }
}

pub struct TestEpochChangeCoreAndLeaderProofsGenerator(
    pub ProofsGeneratorSettings,
    pub Option<ProofOfLeadershipQuotaInputs>,
);

#[async_trait]
impl CoreAndLeaderProofsGenerator for TestEpochChangeCoreAndLeaderProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, _private_inputs: ProofOfCoreQuotaInputs) -> Self {
        Self(settings, None)
    }

    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs) {
        self.0.public_inputs.leader = new_epoch_public;
    }

    fn set_epoch_private(&mut self, new_epoch_private: ProofOfLeadershipQuotaInputs) {
        self.1 = Some(new_epoch_private);
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        None
    }

    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
        None
    }
}

pub struct TestEpochChangeProofsVerifier(
    pub PoQVerificationInputsMinusSigningKey,
    pub Option<LeaderInputs>,
);

#[async_trait]
impl ProofsVerifier for TestEpochChangeProofsVerifier {
    type Error = Infallible;

    fn new(public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self(public_inputs, None)
    }

    fn start_epoch_transition(&mut self, new_pol_inputs: LeaderInputs) {
        self.1 = Some(self.0.leader);
        self.0.leader = new_pol_inputs;
    }

    fn complete_epoch_transition(&mut self) {
        self.1 = None;
    }

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
