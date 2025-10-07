use async_trait::async_trait;
use nomos_blend_message::crypto::proofs::quota::inputs::prove::{
    private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
    public::LeaderInputs,
};
use nomos_core::crypto::ZkHash;

use crate::message_blend::provers::{
    BlendLayerProof, ProofsGeneratorSettings,
    core::{CoreProofsGenerator as _, RealCoreProofsGenerator},
    leader::{LeaderProofsGenerator as _, RealLeaderProofsGenerator},
};

const LOG_TARGET: &str = "blend::scheduling::proofs::core-and-leader";

#[async_trait]
pub trait CoreAndLeaderProofsGenerator: Sized {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self;
    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs);
    fn set_epoch_private(
        &mut self,
        new_epoch_private: ProofOfLeadershipQuotaInputs,
        epoch_nonce: ZkHash,
    );

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof>;
    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof>;
}

enum LeaderProofsGeneratorState {
    NotSet,
    PublicInputsSet(LeaderInputs),
    SecretInputsSet {
        inputs: ProofOfLeadershipQuotaInputs,
        epoch_nonce: ZkHash,
    },
    Set(RealLeaderProofsGenerator),
}

pub struct RealCoreAndLeaderProofsGenerator {
    core_proofs_generator: RealCoreProofsGenerator,
    leader_proofs_generator: LeaderProofsGeneratorState,
}

#[async_trait]
impl CoreAndLeaderProofsGenerator for RealCoreAndLeaderProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self {
        Self {
            core_proofs_generator: RealCoreProofsGenerator::new(settings, private_inputs),
            leader_proofs_generator: LeaderProofsGeneratorState::NotSet,
        }
    }

    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs) {
        tracing::info!(target: LOG_TARGET, "Rotating epoch...");
        self.core_proofs_generator.rotate_epoch(new_epoch_public);

        // If no epoch info is set, an old epoch info is set without an actual proof
        // generator, or a proof generator are already present, override with the new
        // public epoch info. Else, if there are secret inputs set but no
        // generator yet, create a new generator with the public + private info.
        let leader_proofs_generator = match &self.leader_proofs_generator {
            LeaderProofsGeneratorState::NotSet
            | LeaderProofsGeneratorState::PublicInputsSet(_)
            | LeaderProofsGeneratorState::Set(_) => {
                tracing::trace!(target: LOG_TARGET, "Setting new public inputs for epoch rotation.");
                LeaderProofsGeneratorState::PublicInputsSet(new_epoch_public)
            }
            LeaderProofsGeneratorState::SecretInputsSet {
                inputs,
                epoch_nonce,
            } if *epoch_nonce == new_epoch_public.pol_epoch_nonce => {
                tracing::trace!(target: LOG_TARGET, "Setting secret PoL info for a new, transitioned epoch. New leader proofs will be generated.");
                let leader_proofs_generator = RealLeaderProofsGenerator::new(
                    self.core_proofs_generator.settings,
                    inputs.clone(),
                );
                LeaderProofsGeneratorState::Set(leader_proofs_generator)
            }
            LeaderProofsGeneratorState::SecretInputsSet { .. } => {
                LeaderProofsGeneratorState::PublicInputsSet(new_epoch_public)
            }
        };

        self.leader_proofs_generator = leader_proofs_generator;
    }

    fn set_epoch_private(
        &mut self,
        new_epoch_private: ProofOfLeadershipQuotaInputs,
        epoch_nonce: ZkHash,
    ) {
        tracing::info!(target: LOG_TARGET, "Setting epoch secret PoL info...");
        // If no epoch info is set, an old secret epoch info is set without an actual
        // proof generator, or a proof generator are already present, override
        // with the new secret epoch info. Else, if there are public inputs set
        // but no generator yet, create a new generator with the public +
        // private info.
        let leader_proofs_generator = match &self.leader_proofs_generator {
            LeaderProofsGeneratorState::NotSet
            | LeaderProofsGeneratorState::SecretInputsSet { .. }
            | LeaderProofsGeneratorState::Set(_) => {
                tracing::trace!(target: LOG_TARGET, "Setting new secret inputs for epoch rotation.");
                LeaderProofsGeneratorState::SecretInputsSet {
                    epoch_nonce,
                    inputs: new_epoch_private,
                }
            }
            LeaderProofsGeneratorState::PublicInputsSet(public_inputs)
                if epoch_nonce == public_inputs.pol_epoch_nonce =>
            {
                tracing::trace!(target: LOG_TARGET, "Setting public epoch info for a new, transitioned epoch. New leader proofs will be generated.");
                let leader_proofs_generator = RealLeaderProofsGenerator::new(
                    self.core_proofs_generator.settings,
                    new_epoch_private,
                );
                LeaderProofsGeneratorState::Set(leader_proofs_generator)
            }
            LeaderProofsGeneratorState::PublicInputsSet(_) => {
                LeaderProofsGeneratorState::SecretInputsSet {
                    epoch_nonce,
                    inputs: new_epoch_private,
                }
            }
        };

        self.leader_proofs_generator = leader_proofs_generator;
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        self.core_proofs_generator.get_next_proof().await
    }

    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
        let LeaderProofsGeneratorState::Set(leader_proofs_generator) =
            &mut self.leader_proofs_generator
        else {
            return None;
        };
        Some(leader_proofs_generator.get_next_proof().await)
    }
}
