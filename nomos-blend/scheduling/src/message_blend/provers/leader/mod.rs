use core::mem::swap;

use async_trait::async_trait;
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{
            ProofOfQuota,
            inputs::prove::{
                PrivateInputs, PublicInputs, private::ProofOfLeadershipQuotaInputs,
                public::LeaderInputs,
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

const LOG_TARGET: &str = "blend::scheduling::proofs::leader";

/// A `PoQ` generator that deals only with leadership proofs, suitable for edge
/// nodes.
#[async_trait]
pub trait LeaderProofsGenerator: Sized {
    /// Instantiate a new generator with the provided public inputs and secret
    /// `PoL` values.
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfLeadershipQuotaInputs)
    -> Self;
    /// Signal an epoch transition in the middle of the current session, with
    /// new public and secret inputs.
    fn rotate_epoch(
        &mut self,
        new_epoch_public: LeaderInputs,
        new_private_inputs: ProofOfLeadershipQuotaInputs,
    );
    /// Get the next leadership proof.
    async fn get_next_proof(&mut self) -> BlendLayerProof;
}

pub struct RealLeaderProofsGenerator {
    proofs_receiver: Receiver<BlendLayerProof>,
    pub(super) settings: ProofsGeneratorSettings,
    proof_generation_task_handle: Option<JoinHandle<()>>,
}

#[async_trait]
impl LeaderProofsGenerator for RealLeaderProofsGenerator {
    fn new(
        settings: ProofsGeneratorSettings,
        private_inputs: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        let mut self_instance = Self {
            // Will be replaced by the `spawn_new_proof_generation_task` below.
            proofs_receiver: channel(1).1,
            settings,
            proof_generation_task_handle: None,
        };

        self_instance.spawn_new_proof_generation_task(private_inputs);

        self_instance
    }

    fn rotate_epoch(
        &mut self,
        new_epoch_public: LeaderInputs,
        new_private: ProofOfLeadershipQuotaInputs,
    ) {
        tracing::info!(target: LOG_TARGET, "Rotating epoch...");

        // On epoch rotation, we maintain the current session info and only change the
        // PoL relevant parts.
        self.settings.public_inputs.leader = new_epoch_public;

        // Compute new proofs with the updated settings.
        self.spawn_new_proof_generation_task(new_private);
    }

    async fn get_next_proof(&mut self) -> BlendLayerProof {
        self.proofs_receiver
            .recv()
            .await
            .expect("Leadership proof should always be generated.")
    }
}

impl RealLeaderProofsGenerator {
    pub(super) fn terminate_proof_generation_task(&mut self) -> Option<JoinHandle<()>> {
        // Drop the previous channel so we don't get any of the old proofs anymore. This
        // will instruct the spawned task to abort as well.
        swap(&mut self.proofs_receiver, &mut channel(1).1);
        self.proof_generation_task_handle.take()
    }

    fn spawn_new_proof_generation_task(&mut self, private_inputs: ProofOfLeadershipQuotaInputs) {
        // We create a channel that can hold proofs for 2 block proposals. As soon as
        // one set of proofs is generated, a new one is pre-computed.
        let (proofs_sender, proofs_receiver) =
            channel((self.settings.public_inputs.leader.message_quota * 2) as usize);
        self.proof_generation_task_handle = Some(spawn_leader_proof_generation_task(
            proofs_sender,
            self.settings.public_inputs,
            private_inputs,
        ));

        self.proofs_receiver = proofs_receiver;
    }

    #[cfg(test)]
    pub(super) fn rotate_epoch_and_return_old_task(
        &mut self,
        new_epoch_public: LeaderInputs,
        new_private: ProofOfLeadershipQuotaInputs,
    ) -> Option<JoinHandle<()>> {
        let old_handle = self.terminate_proof_generation_task();
        self.rotate_epoch(new_epoch_public, new_private);
        old_handle
    }
}

impl Drop for RealLeaderProofsGenerator {
    fn drop(&mut self) {
        self.terminate_proof_generation_task();
    }
}

fn spawn_leader_proof_generation_task(
    sender_channel: Sender<BlendLayerProof>,
    public_inputs: PoQVerificationInputsMinusSigningKey,
    private_inputs: ProofOfLeadershipQuotaInputs,
) -> JoinHandle<()> {
    spawn_blocking(move || {
        // This task never stops (unless explicitly killed), since we don't know how
        // many proofs are actually needed by a block proposer within an epoch.
        loop {
            for encapsulation_layer in 0..public_inputs.leader.message_quota {
                let ephemeral_signing_key = Ed25519PrivateKey::generate();
                let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                    &PublicInputs {
                        signing_key: ephemeral_signing_key.public_key(),
                        core: public_inputs.core,
                        leader: public_inputs.leader,
                        session: public_inputs.session,
                    },
                    PrivateInputs::new_proof_of_leadership_quota_inputs(
                        encapsulation_layer,
                        private_inputs.clone(),
                    ),
                ) else {
                    tracing::error!(target: LOG_TARGET, "Leadership PoQ generation failed for the provided public and private inputs.");
                    continue;
                };
                let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
                // `blocking_send` will actually stop when the channel is full. Next time a
                // message is to be blended, `N` proofs will be retrieved from the channel,
                // opening the way for more to be generated and pushed.
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
        }
    })
}
