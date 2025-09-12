pub mod crypto;

use async_trait::async_trait;
pub use crypto::CryptographicProcessorSettings;
use futures::{
    channel::mpsc::Receiver,
    future::join,
    stream::{AbortHandle, AbortRegistration, Abortable},
};
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        quota::{
            inputs::prove::{
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs, ProofType},
                PrivateInputs, PublicInputs,
            },
            ProofOfQuota,
        },
        selection::ProofOfSelection,
    },
};
use nomos_core::crypto::ZkHash;
use tokio::{
    runtime::Handle,
    spawn,
    sync::mpsc::{channel, Sender},
    task::AbortHandle,
};

use crate::message_scheduler::session_info::Session;

#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
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

#[derive(Debug)]
pub struct BlendProof {
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
    ephemeral_signing_key: Ed25519PrivateKey,
}

pub trait ProofsGenerator {
    type Error;

    fn new(session_info: SessionInfo) -> Result<Self, Self::Error>;

    async fn get_next_core_proof(&self) -> Option<BlendProof>;
    async fn get_next_leadership_proof(&self) -> Option<BlendProof>;
}

pub struct RealProofsGenerator {
    remaining_core_quota_proofs: u64,
    remaining_leadership_quota_proofs: u64,
    core_proofs_receiver: Receiver<BlendProof>,
    leadership_proofs_receiver: Receiver<BlendProof>,
}

pub enum Error {}

impl ProofsGenerator for RealProofsGenerator {
    type Error = Error;

    fn new(session_info: SessionInfo) -> Result<Self, Self::Error> {
        let (core_proofs_sender, core_proofs_receiver) = channel(session_info.public.core_quota);
        let (leadership_proofs_sender, leadership_proofs_receiver) =
            channel(session_info.public.leader_quota);
        unimplemented!()
    }

    async fn get_next_core_proof(&self) -> Option<BlendProof> {
        unimplemented!()
    }

    async fn get_next_leadership_proof(&self) -> Option<BlendProof> {
        unimplemented!()
    }
}

struct InnerProofsGenerator {
    proofs_generation_task_abort_handle: AbortHandle,
}

impl InnerProofsGenerator {
    pub fn new(
        total_core_proofs: u64,
        total_leadership_proofs: u64,
        core_proofs_sender: Sender<BlendProof>,
        leadership_proofs_sender: Sender<BlendProof>,
        session_info: SessionInfo,
        runtime_handle: Handle,
    ) -> Self {
        let core_proofs_task = spawn(async {
            let session_info = session_info.clone();
            (0..total_core_proofs).for_each(|core_key_index| {
                let ephemeral_signing_key = Ed25519PrivateKey::generate();
                let (proof_of_quota, secret_selection_randomness) = ProofOfQuota::new(
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
                    &PrivateInputs::new_proof_of_core_quota_inputs(
                        core_key_index,
                        ProofOfCoreQuotaInputs {
                            core_path: session_info.private.core_path,
                            core_path_selectors: session_info.private.core_path_selectors,
                            core_sk: session_info.private.core_sk,
                        },
                    ),
                );
                let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
                core_proofs_sender.send(BlendProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key: ephemeral_signing_key,
                })
            });
        });

        let leadership_proofs_task = spawn(async {
            let session_info = session_info.clone();
            (0..total_leadership_proofs).for_each(|leadership_key_index| {
                let ephemeral_signing_key = Ed25519PrivateKey::generate();
                let (proof_of_quota, secret_selection_randomness) = ProofOfQuota::new(
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
                    &PrivateInputs::new_proof_of_leadership_quota_inputs(
                        leadership_key_index,
                        ProofOfLeadershipQuotaInputs {
                            aged_path: session_info.private.aged_path,
                            aged_selector: session_info.private.aged_selector,
                            note_value: session_info.private.note_value,
                            output_number: session_info.private.output_number,
                            pol_secret_key: session_info.private.pol_secret_key,
                            slot: session_info.private.slot,
                            slot_secret: session_info.private.slot_secret,
                            slot_secret_path: session_info.private.slot_secret_path,
                            starting_slot: session_info.private.starting_slot,
                            transaction_hash: session_info.private.transaction_hash,
                        },
                    ),
                );
                let proof_of_selection = ProofOfSelection::new(secret_selection_randomness);
                leadership_proofs_sender.send(BlendProof {
                    proof_of_quota,
                    proof_of_selection,
                    ephemeral_signing_key: ephemeral_signing_key,
                })
            });
        });

        let proofs_generation_task = join(core_proofs_task, leadership_proofs_task);
        let (proofs_generation_task_abort_handle, proofs_generation_task_abort_registration) =
            AbortHandle::new_pair();

        runtime_handle.spawn(Abortable::new(
            proofs_generation_task,
            proofs_generation_task_abort_registration,
        ));

        Self {
            proofs_generation_task_abort_handle,
        }
    }
}
