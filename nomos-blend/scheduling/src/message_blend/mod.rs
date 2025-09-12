pub mod crypto;

use async_trait::async_trait;
pub use crypto::CryptographicProcessorSettings;
use futures::channel::mpsc::Receiver;
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{quota::ProofOfQuota, selection::ProofOfSelection},
};
use tokio::sync::mpsc::channel;

pub struct SessionInfo {
    pub public: PublicInfo,
    pub private: PrivateInfo,
}

pub struct PublicInfo {
    pub session: u64,
    pub core_root: ZkHash,
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
}

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
