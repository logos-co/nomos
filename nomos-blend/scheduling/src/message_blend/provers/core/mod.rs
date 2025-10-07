use async_trait::async_trait;
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            quota::{
                ProofOfQuota,
                inputs::prove::{
                    PrivateInputs, PublicInputs, private::ProofOfCoreQuotaInputs,
                    public::LeaderInputs,
                },
            },
            selection::ProofOfSelection,
        },
    },
    encap::encapsulated::PoQVerificationInputsMinusSigningKey,
};
use tokio::{
    sync::{
        mpsc::{Receiver, Sender, channel},
        oneshot,
    },
    task::spawn_blocking,
};

use crate::message_blend::provers::{BlendLayerProof, ProofsGeneratorSettings};

const LOG_TARGET: &str = "blend::scheduling::proofs::core";

#[async_trait]
pub trait CoreProofsGenerator: Sized {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self;
    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs);
    async fn get_next_proof(&mut self) -> Option<BlendLayerProof>;
}

pub struct RealCoreProofsGenerator {
    remaining_quota: u64,
    proofs_receiver: Receiver<BlendLayerProof>,
    proof_generation_task_termination_sender: Option<oneshot::Sender<()>>,
    pub(super) settings: ProofsGeneratorSettings,
    pub(super) private_inputs: ProofOfCoreQuotaInputs,
}

#[async_trait]
impl CoreProofsGenerator for RealCoreProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, private_inputs: ProofOfCoreQuotaInputs) -> Self {
        let mut self_instance = Self {
            private_inputs,
            proof_generation_task_termination_sender: None,
            // Will be replaced by the `spawn_new_proof_generation_task` below.
            proofs_receiver: channel(1).1,
            remaining_quota: settings.public_inputs.core.quota,
            settings,
        };

        self_instance.spawn_new_proof_generation_task();

        self_instance
    }

    fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs) {
        tracing::info!(target: LOG_TARGET, "Rotating epoch...");
        self.terminate_proof_generation_task();

        // On epoch rotation, we maintain the remaining session quota for core proofs
        // and we only update the PoL part of the public inputs, before regenerating all
        // proofs.
        let settings = {
            let mut settings = self.settings;
            settings.public_inputs.leader = new_epoch_public;
            settings.public_inputs.core.quota = self.remaining_quota;
            settings
        };
        self.settings = settings;

        // Compute new proofs with the updated settings.
        self.spawn_new_proof_generation_task();
    }

    async fn get_next_proof(&mut self) -> Option<BlendLayerProof> {
        self.remaining_quota = self.remaining_quota.checked_sub(1)?;
        self.proofs_receiver.recv().await
    }
}

impl RealCoreProofsGenerator {
    fn terminate_proof_generation_task(&mut self) {
        let Some(proof_generation_task_termination_sender) =
            self.proof_generation_task_termination_sender.take()
        else {
            tracing::debug!(target: LOG_TARGET, "No outstanding task to terminate.");
            return;
        };
        if proof_generation_task_termination_sender.send(()).is_err() {
            tracing::debug!(target: LOG_TARGET, "Outstanding task has already terminated on its own.");
        } else {
            tracing::debug!(target: LOG_TARGET, "Outstanding task killed.");
        }
    }

    fn spawn_new_proof_generation_task(&mut self) {
        let (proofs_sender, proofs_receiver) = channel(self.remaining_quota as usize);
        let (proof_generation_task_termination_sender, proof_generation_task_termination_receiver) =
            oneshot::channel();
        spawn_core_proof_generation_task(
            proofs_sender,
            self.settings.public_inputs,
            self.private_inputs.clone(),
            proof_generation_task_termination_receiver,
        );

        self.proofs_receiver = proofs_receiver;
        self.proof_generation_task_termination_sender =
            Some(proof_generation_task_termination_sender);
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
    termination_receiver: oneshot::Receiver<()>,
) {
    spawn_blocking(move || {
        tracing::trace!(target: LOG_TARGET, "Generating {} core quota proofs", public_inputs.core.quota);
        for key_index in 0..public_inputs.core.quota {
            if !termination_receiver.is_empty() {
                tracing::debug!(target: LOG_TARGET, "Aborting core quota generation task");
                return;
            }
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
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            if let Err(err) = sender_channel.blocking_send(BlendLayerProof {
                proof_of_quota,
                proof_of_selection,
                ephemeral_signing_key,
            }) {
                tracing::error!(target: LOG_TARGET, "Failed to send proof to consumer. Error: {err:?}");
            }
        }
    });
}
