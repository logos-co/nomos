use core::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt as _};
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{
            ProofOfQuota,
            inputs::prove::{
                PrivateInputs, PublicInputs, private::ProofOfCoreQuotaInputs, public::LeaderInputs,
            },
        },
        selection::ProofOfSelection,
    },
};
use tokio::task::spawn_blocking;

use crate::message_blend::provers::{BlendLayerProof, ProofsGeneratorSettings};

#[cfg(test)]
mod tests;

const LOG_TARGET: &str = "blend::scheduling::proofs::core";
const PROOFS_GENERATOR_BUFFER_SIZE: usize = 10;

/// Proof generator for core `PoQ` variants.
#[async_trait]
pub trait CoreProofsGenerator: Sized {
    /// Instantiate a new generator for the duration of a session.
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self;
    /// Notify the proof generator that a new epoch has started mid-session.
    /// This will trigger proof re-generation due to the change in the set of
    /// public inputs.
    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs);
    /// Request a new core proof from the prover. It returns `None` if the
    /// maximum core quota has already been reached for this session.
    async fn get_next_proof(&mut self) -> Option<BlendLayerProof>;
}

pub struct RealCoreProofsGenerator {
    remaining_quota: u64,
    pub(super) settings: ProofsGeneratorSettings,
    pub(super) private_inputs: ProofOfCoreQuotaInputs,
    proof_stream: Pin<Box<dyn Stream<Item = BlendLayerProof> + Send + Sync>>,
}

#[async_trait]
impl CoreProofsGenerator for RealCoreProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self {
        Self {
            private_inputs: private_inputs.clone(),
            proof_stream: Box::pin(create_proof_stream(
                settings.public_inputs,
                private_inputs,
                0,
            )),
            remaining_quota: settings.public_inputs.core.quota,
            settings,
        }
    }

    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs) {
        tracing::info!(target: LOG_TARGET, "Rotating epoch...");

        // On epoch rotation, we maintain the remaining session quota for core proofs
        // and we only update the PoL part of the public inputs, before regenerating all
        // proofs.
        self.settings.public_inputs.leader = new_epoch_public;
        let next_key_index = self
            .settings
            .public_inputs
            .core
            .quota
            .checked_sub(self.remaining_quota)
            .expect("Remaining quota should never be larger than total quota.");

        // Compute new proofs with the updated settings.
        self.generate_new_proofs_stream(next_key_index);
    }

    async fn get_next_proof(&mut self) -> Option<BlendLayerProof> {
        self.remaining_quota = self.remaining_quota.checked_sub(1)?;
        let proof = self.proof_stream.next().await?;
        tracing::trace!(target: LOG_TARGET, "Generated core Blend layer proof with key nullifier {:?} addressed to node at index {:?}", proof.proof_of_quota.key_nullifier(), proof.proof_of_selection.expected_index(self.settings.membership_size));
        Some(proof)
    }
}

impl RealCoreProofsGenerator {
    // This will kill the previous running task, if any, since we swap the receiver
    // channel, hence the old task will fail to send new proofs and will abort on
    // its own.
    fn generate_new_proofs_stream(&mut self, starting_key_index: u64) {
        if self.remaining_quota == 0 {
            return;
        }

        self.proof_stream = Box::pin(create_proof_stream(
            self.settings.public_inputs,
            self.private_inputs.clone(),
            starting_key_index,
        ));
    }
}

fn create_proof_stream(
    public_inputs: PoQVerificationInputsMinusSigningKey,
    private_inputs: ProofOfCoreQuotaInputs,
    starting_key_index: u64,
) -> impl Stream<Item = BlendLayerProof> {
    let proofs_to_generate = public_inputs
        .core
        .quota
        .checked_sub(starting_key_index)
        .expect("Starting key index should never be larger than core quota.");
    tracing::trace!(target: LOG_TARGET, "Generating {proofs_to_generate} core quota proofs starting from index: {starting_key_index}.");

    let quota = public_inputs.core.quota;
    stream::iter(starting_key_index..quota)
        .map(move |key_index| {
            let private_inputs = private_inputs.clone();

            spawn_blocking(move || {
                let ephemeral_signing_key = Ed25519PrivateKey::generate();
                let (proof_of_quota, secret_selection_randomness) = ProofOfQuota::new(
                    &PublicInputs {
                        signing_key: ephemeral_signing_key.public_key(),
                        core: public_inputs.core,
                        leader: public_inputs.leader,
                        session: public_inputs.session,
                    },
                    PrivateInputs::new_proof_of_core_quota_inputs(key_index, private_inputs),
                )
                .ok()?;
                let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
                Some(BlendLayerProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
            })
        })
        .buffered(PROOFS_GENERATOR_BUFFER_SIZE)
        .filter_map(|result| async { result.ok().flatten() })
}
