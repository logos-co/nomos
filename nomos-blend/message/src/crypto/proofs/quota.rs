use nomos_core::crypto::ZkHash;
use serde::{Deserialize, Serialize};

use crate::crypto::keys::Ed25519PublicKey;

pub const PROOF_OF_QUOTA_SIZE: usize = 160;

pub struct PublicInputs {
    pub session_number: u64,
    pub core_quota: usize,
    pub leader_quota: usize,
    pub core_root: ZkHash,
    pub signing_key: Ed25519PublicKey,
    pub pol_epoch_nonce: u64,
    pub pol_t0: u64,
    pub pol_t1: u64,
    pub pol_ledger_aged: ZkHash,
}

#[derive(Default)]
pub struct ProofOfCoreQuotaPrivateInputs {
    pub core_sk: ZkHash,
    pub core_path: Vec<ZkHash>,
    pub core_path_selectors: Vec<bool>,
}

#[derive(Default)]
pub struct ProofOfLeadershipQuotaPrivateInputs {
    pub pol_sl: u64,
    pub pol_sk_starting_slot: u64,
    pub pol_note_value: u64,
    pub pol_note_tx_hash: ZkHash,
    pub pol_note_output_number: u64,
    pub pol_noteid_path: Vec<ZkHash>,
    pub pol_noteid_path_selectors: Vec<bool>,
    pub pol_slot_secret: u64,
    pub pol_slot_secret_path: Vec<ZkHash>,
}

pub struct PrivateInputs {
    key_index: usize,
    selector: bool,
    proof_of_core_quota: ProofOfCoreQuotaPrivateInputs,
    proof_of_leadership_quota: ProofOfLeadershipQuotaPrivateInputs,
}

impl PrivateInputs {
    #[must_use]
    pub fn new_proof_of_core_quota_inputs(
        key_index: usize,
        proof_of_core_quota_inputs: ProofOfCoreQuotaPrivateInputs,
    ) -> Self {
        Self {
            key_index,
            selector: false,
            proof_of_core_quota: proof_of_core_quota_inputs,
            proof_of_leadership_quota: ProofOfLeadershipQuotaPrivateInputs::default(),
        }
    }

    #[must_use]
    pub fn new_proof_of_leadership_quota_inputs(
        key_index: usize,
        proof_of_leadership_quota_inputs: ProofOfLeadershipQuotaPrivateInputs,
    ) -> Self {
        Self {
            key_index,
            selector: true,
            proof_of_core_quota: ProofOfCoreQuotaPrivateInputs::default(),
            proof_of_leadership_quota: proof_of_leadership_quota_inputs,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ProofOfQuota(#[serde(with = "serde_big_array::BigArray")] [u8; PROOF_OF_QUOTA_SIZE]);

impl ProofOfQuota {
    // TODO: Remove this once the actual proof of quota is implemented.
    #[must_use]
    pub const fn dummy() -> Self {
        Self([6u8; PROOF_OF_QUOTA_SIZE])
    }

    #[must_use]
    pub fn new(_public_inputs: PublicInputs, _private_inputs: PrivateInputs) -> (Self, ZkHash) {
        // TODO: Interact with circom circuit generation.
        (Self::dummy(), ZkHash::default())
    }

    pub fn verify(self, _public_inputs: &PublicInputs) -> Result<ZkHash, ()> {
        // TODO: Interact with circom circuit verification.
        Ok(ZkHash::default())
    }
}

impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        Self(bytes)
    }
}
