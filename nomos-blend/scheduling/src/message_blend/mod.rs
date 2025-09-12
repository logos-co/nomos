pub mod crypto;

use async_trait::async_trait;
pub use crypto::CryptographicProcessorSettings;
use futures::{
    future::join,
    stream::{AbortHandle, Abortable},
};
use nomos_blend_message::crypto::{
    keys::{Ed25519PrivateKey, Ed25519PublicKey},
    proofs::{
        quota::{
            inputs::prove::{
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
                PrivateInputs, PublicInputs,
            },
            ProofOfQuota,
        },
        selection::ProofOfSelection,
    },
};
use nomos_core::crypto::ZkHash;
use tokio::{
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
    task::spawn_blocking,
};

#[derive(Clone)]
pub struct SessionInfo {
    pub public: PublicInfo,
    pub private: PrivateInfo,
}

#[derive(Clone, Copy)]
pub struct PublicInfo {
    pub session: u64,
    pub core_root: ZkHash,
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
}

#[derive(Clone)]
pub struct PrivateInfo {
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

pub struct BlendProof {
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
    ephemeral_signing_key: Ed25519PrivateKey,
}

#[async_trait]
pub trait ProofsGenerator: Sized {
    fn new(session_info: SessionInfo) -> Self;

    async fn get_next_core_proof(&mut self) -> Option<BlendProof>;
    async fn get_next_leadership_proof(&mut self) -> Option<BlendProof>;
}

pub struct RealProofsGenerator {
    remaining_core_quota_proofs: u64,
    remaining_leadership_quota_proofs: u64,
    core_proofs_receiver: Receiver<BlendProof>,
    leadership_proofs_receiver: Receiver<BlendProof>,
    proofs_generation_task_abort_handle: AbortHandle,
}

#[async_trait]
impl ProofsGenerator for RealProofsGenerator {
    fn new(session_info: SessionInfo) -> Self {
        let core_quota = session_info.public.core_quota;
        let leadership_quota = session_info.public.leader_quota;
        let (core_proofs_sender, core_proofs_receiver) = channel(core_quota as usize);
        let (leadership_proofs_sender, leadership_proofs_receiver) =
            channel(leadership_quota as usize);
        Self {
            remaining_core_quota_proofs: core_quota,
            remaining_leadership_quota_proofs: leadership_quota,
            core_proofs_receiver,
            leadership_proofs_receiver,
            proofs_generation_task_abort_handle: start(
                session_info.public.core_quota,
                session_info.public.leader_quota,
                core_proofs_sender,
                leadership_proofs_sender,
                session_info,
            ),
        }
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendProof> {
        let new_remaining_core_quota = self.remaining_core_quota_proofs.checked_sub(1)?;
        self.remaining_core_quota_proofs = new_remaining_core_quota;
        self.core_proofs_receiver.recv().await
    }

    async fn get_next_leadership_proof(&mut self) -> Option<BlendProof> {
        let new_remaining_leadership_quota =
            self.remaining_leadership_quota_proofs.checked_sub(1)?;
        self.remaining_leadership_quota_proofs = new_remaining_leadership_quota;
        self.leadership_proofs_receiver.recv().await
    }
}

impl Drop for RealProofsGenerator {
    fn drop(&mut self) {
        self.proofs_generation_task_abort_handle.abort();
    }
}

fn start(
    total_core_proofs: u64,
    total_leadership_proofs: u64,
    core_proofs_sender: Sender<BlendProof>,
    leadership_proofs_sender: Sender<BlendProof>,
    session_info: SessionInfo,
) -> AbortHandle {
    let session_info_clone = session_info.clone();
    let core_proofs_task = spawn_blocking(async move || {
        for core_key_index in 0..total_core_proofs {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &PublicInputs {
                    core_quota: session_info_clone.public.core_quota,
                    core_root: session_info_clone.public.core_root,
                    leader_quota: session_info_clone.public.leader_quota,
                    pol_epoch_nonce: session_info_clone.public.pol_epoch_nonce,
                    pol_ledger_aged: session_info_clone.public.pol_ledger_aged,
                    session: session_info_clone.public.session,
                    signing_key: ephemeral_signing_key.public_key(),
                    total_stake: session_info_clone.public.total_stake,
                },
                PrivateInputs::new_proof_of_core_quota_inputs(
                    core_key_index,
                    ProofOfCoreQuotaInputs {
                        core_path: session_info_clone.private.core_path.clone(),
                        core_path_selectors: session_info_clone.private.core_path_selectors.clone(),
                        core_sk: session_info_clone.private.core_sk,
                    },
                ),
            ) else {
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            core_proofs_sender
                .send(BlendProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key,
                })
                .await
                .unwrap();
        }
    });

    let leadership_proofs_task = spawn_blocking(async move || {
        for leadership_key_index in 0..total_leadership_proofs {
            let ephemeral_signing_key = Ed25519PrivateKey::generate();
            let Ok((proof_of_quota, secret_selection_randomness)) = ProofOfQuota::new(
                &PublicInputs {
                    core_quota: session_info.public.core_quota,
                    core_root: session_info.public.core_root,
                    leader_quota: session_info.public.leader_quota,
                    pol_epoch_nonce: session_info.public.pol_epoch_nonce,
                    pol_ledger_aged: session_info.public.pol_ledger_aged,
                    session: session_info.public.session,
                    signing_key: ephemeral_signing_key.public_key(),
                    total_stake: session_info.public.total_stake,
                },
                PrivateInputs::new_proof_of_leadership_quota_inputs(
                    leadership_key_index,
                    ProofOfLeadershipQuotaInputs {
                        aged_path: session_info.private.aged_path.clone(),
                        aged_selector: session_info.private.aged_selector.clone(),
                        note_value: session_info.private.note_value,
                        output_number: session_info.private.output_number,
                        pol_secret_key: session_info.private.pol_secret_key,
                        slot: session_info.private.slot,
                        slot_secret: session_info.private.slot_secret,
                        slot_secret_path: session_info.private.slot_secret_path.clone(),
                        starting_slot: session_info.private.starting_slot,
                        transaction_hash: session_info.private.transaction_hash,
                    },
                ),
            ) else {
                continue;
            };
            let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
            leadership_proofs_sender
                .send(BlendProof {
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
