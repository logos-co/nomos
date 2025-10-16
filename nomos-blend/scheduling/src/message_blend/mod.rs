use async_trait::async_trait;
pub use crypto::SessionCryptographicProcessorSettings;
use futures::{
    future::join,
    stream::{AbortHandle, Abortable},
};
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            quota::{
                self, ProofOfQuota,
                inputs::prove::private::{
                    ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs, ProofType,
                },
            },
            selection::ProofOfSelection,
        },
    },
    encap::encapsulated::PoQVerificationInputMinusSigningKey,
};
use nomos_core::crypto::ZkHash;
use poq::{CorePathAndSelectors, NotePathAndSelectors, SlotSecretPath};
use tokio::{
    spawn,
    sync::mpsc::{Receiver, Sender, channel},
    task::{JoinHandle, spawn_blocking},
};

pub mod crypto;

#[cfg(test)]
mod tests;

/// Information about the ongoing session required to build `PoQ`s and
/// `PoSel`s.
#[derive(Clone)]
pub struct SessionInfo {
    /// Public session info.
    pub public_inputs: PublicInputs,
    /// Private session info.
    pub private_inputs: PrivateInputs,
    /// If the local node is a core node, its index.
    pub local_node_index: Option<usize>,
    /// Size of membership set for the current session.
    pub membership_size: usize,
}

impl From<SessionInfo> for PoQVerificationInputMinusSigningKey {
    fn from(SessionInfo { public_inputs, .. }: SessionInfo) -> Self {
        public_inputs.into()
    }
}

#[derive(Clone, Copy)]
pub struct PublicInputs {
    pub session: u64,
    pub core_root: ZkHash,
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
}

impl From<PublicInputs> for PoQVerificationInputMinusSigningKey {
    fn from(
        PublicInputs {
            core_quota,
            core_root,
            leader_quota,
            pol_epoch_nonce,
            pol_ledger_aged,
            session,
            total_stake,
        }: PublicInputs,
    ) -> Self {
        Self {
            core_quota,
            core_root,
            leader_quota,
            pol_epoch_nonce,
            pol_ledger_aged,
            session,
            total_stake,
        }
    }
}

#[derive(Clone)]
pub struct PrivateInputs {
    pub core_sk: ZkHash,
    pub core_path_and_selectors: CorePathAndSelectors,
    pub slot: u64,
    pub note_value: u64,
    pub transaction_hash: ZkHash,
    pub output_number: u64,
    pub aged_path_and_selectors: NotePathAndSelectors,
    pub slot_secret: ZkHash,
    pub slot_secret_path: SlotSecretPath,
    pub starting_slot: u64,
    pub pol_secret_key: ZkHash,
}

/// A single proof to be attached to one layer of a Blend message.
pub struct BlendLayerProof {
    /// `PoQ`
    pub proof_of_quota: ProofOfQuota,
    /// `PoSel`
    pub proof_of_selection: ProofOfSelection,
    /// Ephemeral key used to sign the message layer's payload.
    pub ephemeral_signing_key: Ed25519PrivateKey,
}

/// A trait to generate core and leadership `PoQs`.
#[async_trait]
pub trait ProofsGenerator: Sized {
    /// Initialize the proof generator with the current session information.
    fn new(session_info: SessionInfo) -> Self;

    /// Get or generate the next core `PoQ`, if the maximum allowance has not
    /// been reached.
    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof>;
    /// Get or generate the next leadership `PoQ`, if the maximum allowance has
    /// not been reached.
    async fn get_next_leadership_proof(&mut self) -> Option<BlendLayerProof>;
}

/// An implementor of `ProofsGenerator` that interacts with the actual proofs
/// types and their underlying Circom circuits.
///
/// Core and leadership proofs are generated in parallel in two different Tokio tasks. The task is spawned when the proof creator is instantiated, as suggested by the Blend v1 spec <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81d2853ee0bd41e2ae1b>.
pub struct RealProofsGenerator {
    remaining_core_quota_proofs: u64,
    remaining_leadership_quota_proofs: u64,
    core_proofs_receiver: Receiver<BlendLayerProof>,
    leadership_proofs_receiver: Receiver<BlendLayerProof>,
    proofs_generation_task_abort_handle: AbortHandle,
}

#[async_trait]
impl ProofsGenerator for RealProofsGenerator {
    fn new(session_info: SessionInfo) -> Self {
        let core_quota = session_info.public_inputs.core_quota;
        let leadership_quota = session_info.public_inputs.leader_quota;
        let (core_proofs_sender, core_proofs_receiver) = channel(core_quota as usize);
        let (leadership_proofs_sender, leadership_proofs_receiver) =
            channel(leadership_quota as usize);
        Self {
            remaining_core_quota_proofs: core_quota,
            remaining_leadership_quota_proofs: leadership_quota,
            core_proofs_receiver,
            leadership_proofs_receiver,
            proofs_generation_task_abort_handle: start(
                core_proofs_sender,
                leadership_proofs_sender,
                session_info,
            ),
        }
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        self.remaining_core_quota_proofs = self.remaining_core_quota_proofs.checked_sub(1)?;
        self.core_proofs_receiver.recv().await
    }

    async fn get_next_leadership_proof(&mut self) -> Option<BlendLayerProof> {
        self.remaining_leadership_quota_proofs =
            self.remaining_leadership_quota_proofs.checked_sub(1)?;
        self.leadership_proofs_receiver.recv().await
    }
}

impl Drop for RealProofsGenerator {
    fn drop(&mut self) {
        self.proofs_generation_task_abort_handle.abort();
    }
}

// Start the two tasks to generate core and leadership proofs. It internally
// uses `spawn_blocking` since we run a loop until all necessary proofs have
// been pre-computed, so that the rest of the session can proceed smoothly.
fn start(
    core_proofs_sender: Sender<BlendLayerProof>,
    leadership_proofs_sender: Sender<BlendLayerProof>,
    SessionInfo {
        public_inputs,
        private_inputs:
            PrivateInputs {
                aged_path_and_selectors,
                core_path_and_selectors,
                core_sk,
                note_value,
                output_number,
                pol_secret_key,
                slot,
                slot_secret,
                slot_secret_path,
                starting_slot,
                transaction_hash,
            },
        ..
    }: SessionInfo,
) -> AbortHandle {
    let core_proof_inputs = ProofOfCoreQuotaInputs {
        core_path_and_selectors,
        core_sk,
    };
    let core_proofs_task =
        spawn_proof_generation_task(core_proofs_sender, public_inputs, core_proof_inputs.into());
    let leadership_proof_inputs = ProofOfLeadershipQuotaInputs {
        aged_path_and_selectors,
        note_value,
        output_number,
        pol_secret_key,
        slot,
        slot_secret,
        slot_secret_path,
        starting_slot,
        transaction_hash,
    };
    let leadership_proofs_task = spawn_proof_generation_task(
        leadership_proofs_sender,
        public_inputs,
        leadership_proof_inputs.into(),
    );

    let proofs_generation_task = join(core_proofs_task, leadership_proofs_task);
    let (proofs_generation_task_abort_handle, proofs_generation_task_abort_registration) =
        AbortHandle::new_pair();

    spawn(Abortable::new(
        proofs_generation_task,
        proofs_generation_task_abort_registration,
    ));

    proofs_generation_task_abort_handle
}

fn spawn_proof_generation_task(
    sender_channel: Sender<BlendLayerProof>,
    public_inputs: PublicInputs,
    proof_type: ProofType,
) -> JoinHandle<()> {
    let quota = match proof_type {
        ProofType::CoreQuota(_) => public_inputs.core_quota,
        ProofType::LeadershipQuota(_) => public_inputs.leader_quota,
    };
    spawn_blocking(move || {
        for key_index in 0..quota {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let private_inputs = match &proof_type {
                ProofType::CoreQuota(private_core_quota_inputs) => {
                    quota::inputs::prove::PrivateInputs::new_proof_of_core_quota_inputs(
                        key_index,
                        *private_core_quota_inputs.clone(),
                    )
                }
                ProofType::LeadershipQuota(private_leadership_quota_inputs) => {
                    quota::inputs::prove::PrivateInputs::new_proof_of_leadership_quota_inputs(
                        key_index,
                        *private_leadership_quota_inputs.clone(),
                    )
                }
            };
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &quota::inputs::prove::PublicInputs {
                    core_quota: public_inputs.core_quota,
                    core_root: public_inputs.core_root,
                    leader_quota: public_inputs.leader_quota,
                    pol_epoch_nonce: public_inputs.pol_epoch_nonce,
                    pol_ledger_aged: public_inputs.pol_ledger_aged,
                    session: public_inputs.session,
                    signing_key: ephemeral_signing_key.public_key(),
                    total_stake: public_inputs.total_stake,
                },
                private_inputs,
            ) else {
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            sender_channel
                .blocking_send(BlendLayerProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
                .unwrap();
        }
    })
}
