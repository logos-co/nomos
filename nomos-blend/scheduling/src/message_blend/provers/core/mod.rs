use core::mem::swap;

use async_trait::async_trait;
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
use tokio::{
    sync::mpsc::{Receiver, Sender, channel},
    task::{JoinHandle, spawn_blocking},
};

use crate::message_blend::provers::{BlendLayerProof, ProofsGeneratorSettings};

#[cfg(test)]
mod tests;

const LOG_TARGET: &str = "blend::scheduling::proofs::core";

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
    proofs_receiver: Receiver<BlendLayerProof>,
    pub(super) settings: ProofsGeneratorSettings,
    pub(super) private_inputs: ProofOfCoreQuotaInputs,
    proof_generation_task_handle: Option<JoinHandle<()>>,
}

#[async_trait]
impl CoreProofsGenerator for RealCoreProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self {
        let mut self_instance = Self {
            private_inputs,
            // Will be replaced by the `spawn_new_proof_generation_task` below.
            proofs_receiver: channel(1).1,
            remaining_quota: settings.public_inputs.core.quota,
            settings,
            proof_generation_task_handle: None,
        };

        self_instance.spawn_new_proof_generation_task(0);

        self_instance
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
        self.spawn_new_proof_generation_task(next_key_index);
    }

    async fn get_next_proof(&mut self) -> Option<BlendLayerProof> {
        self.remaining_quota = self.remaining_quota.checked_sub(1)?;
        self.proofs_receiver.recv().await
    }
}

impl RealCoreProofsGenerator {
    fn terminate_proof_generation_task(&mut self) -> Option<JoinHandle<()>> {
        // Drop the previous channel so we don't get any of the old proofs anymore. This
        // will instruct the spawned task to abort as well.
        swap(&mut self.proofs_receiver, &mut channel(1).1);
        self.proof_generation_task_handle.take()
    }

    // This will kill the previous running task, if any, since we swap the receiver
    // channel, hence the old task will fail to send new proofs and will abort on
    // its own.
    fn spawn_new_proof_generation_task(&mut self, starting_key_index: u64) {
        if self.remaining_quota == 0 {
            return;
        }

        let (proofs_sender, proofs_receiver) = channel(self.remaining_quota as usize);
        self.proof_generation_task_handle = Some(spawn_core_proof_generation_task(
            proofs_sender,
            self.settings.public_inputs,
            self.private_inputs.clone(),
            starting_key_index,
        ));

        self.proofs_receiver = proofs_receiver;
    }

    #[cfg(test)]
    pub(super) fn rotate_epoch_and_return_old_task(
        &mut self,
        new_epoch_public: LeaderInputs,
    ) -> Option<JoinHandle<()>> {
        let old_handle = self.terminate_proof_generation_task();
        self.rotate_epoch(new_epoch_public);
        old_handle
    }
}

impl Drop for RealCoreProofsGenerator {
    fn drop(&mut self) {
        self.terminate_proof_generation_task();
    }
}

fn spawn_core_proof_generation_task(
    sender_channel: Sender<BlendLayerProof>,
    public_inputs: PoQVerificationInputsMinusSigningKey,
    private_inputs: ProofOfCoreQuotaInputs,
    starting_key_index: u64,
) -> JoinHandle<()> {
    spawn_blocking(move || {
        let proofs_to_generate = public_inputs
            .core
            .quota
            .checked_sub(starting_key_index)
            .expect("Starting key index should never be larger than core quota.");
        tracing::trace!(target: LOG_TARGET, "Generating {proofs_to_generate} core quota proofs starting from index: {starting_key_index}.");
        for key_index in starting_key_index..public_inputs.core.quota {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &PublicInputs {
                    signing_key: ephemeral_signing_key.public_key(),
                    core: public_inputs.core,
                    leader: public_inputs.leader,
                    session: public_inputs.session,
                },
                PrivateInputs::new_proof_of_core_quota_inputs(key_index, private_inputs.clone()),
            ) else {
                tracing::error!(target: LOG_TARGET, "Core PoQ generation failed for the provided public and private inputs.");
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            if sender_channel
                .blocking_send(BlendLayerProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
                .is_err()
            {
                tracing::debug!(target: LOG_TARGET, "Failed to send proof to consumer due to channel being dropped. Aborting...");
                return;
            }
        }
    })
}
