pub mod crypto;

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
                inputs::prove::private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
            },
            selection::ProofOfSelection,
        },
    },
    encap::encapsulated::PoQVerificationInputMinusSigningKey,
};
use nomos_core::crypto::ZkHash;
use tokio::{
    spawn,
    sync::mpsc::{Receiver, Sender, channel},
    task::spawn_blocking,
};

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
    pub core_path: Vec<ZkHash>,
    pub core_path_selectors: Vec<bool>,
    pub slot: u64,
    pub note_value: u64,
    pub transaction_hash: ZkHash,
    pub output_number: u64,
    pub aged_path: Vec<ZkHash>,
    pub aged_selector: Vec<bool>,
    pub slot_secret: ZkHash,
    pub slot_secret_path: Vec<ZkHash>,
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
    session_info: SessionInfo,
) -> AbortHandle {
    let session_info_clone = session_info.clone();
    let total_core_proofs = session_info.public_inputs.core_quota;
    let core_proofs_task = spawn_blocking(async move || {
        for core_key_index in 0..total_core_proofs {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &quota::inputs::prove::PublicInputs {
                    core_quota: session_info_clone.public_inputs.core_quota,
                    core_root: session_info_clone.public_inputs.core_root,
                    leader_quota: session_info_clone.public_inputs.leader_quota,
                    pol_epoch_nonce: session_info_clone.public_inputs.pol_epoch_nonce,
                    pol_ledger_aged: session_info_clone.public_inputs.pol_ledger_aged,
                    session: session_info_clone.public_inputs.session,
                    signing_key: ephemeral_signing_key.public_key(),
                    total_stake: session_info_clone.public_inputs.total_stake,
                },
                quota::inputs::prove::PrivateInputs::new_proof_of_core_quota_inputs(
                    core_key_index,
                    ProofOfCoreQuotaInputs {
                        core_path: session_info_clone.private_inputs.core_path.clone(),
                        core_path_selectors: session_info_clone
                            .private_inputs
                            .core_path_selectors
                            .clone(),
                        core_sk: session_info_clone.private_inputs.core_sk,
                    },
                ),
            ) else {
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            core_proofs_sender
                .send(BlendLayerProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
                .await
                .unwrap();
        }
    });

    let total_leadership_proofs = session_info.public_inputs.leader_quota;
    let leadership_proofs_task = spawn_blocking(async move || {
        for leadership_key_index in 0..total_leadership_proofs {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &quota::inputs::prove::PublicInputs {
                    core_quota: session_info.public_inputs.core_quota,
                    core_root: session_info.public_inputs.core_root,
                    leader_quota: session_info.public_inputs.leader_quota,
                    pol_epoch_nonce: session_info.public_inputs.pol_epoch_nonce,
                    pol_ledger_aged: session_info.public_inputs.pol_ledger_aged,
                    session: session_info.public_inputs.session,
                    signing_key: ephemeral_signing_key.public_key(),
                    total_stake: session_info.public_inputs.total_stake,
                },
                quota::inputs::prove::PrivateInputs::new_proof_of_leadership_quota_inputs(
                    leadership_key_index,
                    ProofOfLeadershipQuotaInputs {
                        aged_path: session_info.private_inputs.aged_path.clone(),
                        aged_selector: session_info.private_inputs.aged_selector.clone(),
                        note_value: session_info.private_inputs.note_value,
                        output_number: session_info.private_inputs.output_number,
                        pol_secret_key: session_info.private_inputs.pol_secret_key,
                        slot: session_info.private_inputs.slot,
                        slot_secret: session_info.private_inputs.slot_secret,
                        slot_secret_path: session_info.private_inputs.slot_secret_path.clone(),
                        starting_slot: session_info.private_inputs.starting_slot,
                        transaction_hash: session_info.private_inputs.transaction_hash,
                    },
                ),
            ) else {
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            leadership_proofs_sender
                .send(BlendLayerProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
                .await
                .unwrap();
        }
    });

    let proofs_generation_task = join(core_proofs_task, leadership_proofs_task);
    let (proofs_generation_task_abort_handle, proofs_generation_task_abort_registration) =
        AbortHandle::new_pair();

    spawn(Abortable::new(
        proofs_generation_task,
        proofs_generation_task_abort_registration,
    ));

    proofs_generation_task_abort_handle
}
