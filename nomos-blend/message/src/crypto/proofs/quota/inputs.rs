use nomos_core::crypto::ZkHash;

use crate::crypto::keys::Ed25519PublicKey;

/// Public inputs for all types of Proof of Quota. Spec defined at: https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81fd9bf3fbb0eededca5.
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

/// Private inputs for Proof of Core Quota. Spec: https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81a18576f67b910d34d4.
#[derive(Default)]
pub struct ProofOfCoreQuotaPrivateInputs {
    pub core_sk: ZkHash,
    pub core_path: Vec<ZkHash>,
    pub core_path_selectors: Vec<bool>,
}

/// Private inputs for Proof of Leadership Quota. Spec: https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81a18576f67b910d34d4.
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

enum PrivateInputsType {
    CoreQuota(ProofOfCoreQuotaPrivateInputs),
    LeadershipQuota(ProofOfLeadershipQuotaPrivateInputs),
}

impl From<ProofOfCoreQuotaPrivateInputs> for PrivateInputsType {
    fn from(value: ProofOfCoreQuotaPrivateInputs) -> Self {
        Self::CoreQuota(value)
    }
}

impl From<ProofOfLeadershipQuotaPrivateInputs> for PrivateInputsType {
    fn from(value: ProofOfLeadershipQuotaPrivateInputs) -> Self {
        Self::LeadershipQuota(value)
    }
}

/// Private inputs for all types of Proof of Quota. Spec: https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81a18576f67b910d34d4.
pub struct PrivateInputs {
    key_index: usize,
    selector: bool,
    private_inputs: PrivateInputsType,
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
            private_inputs: proof_of_core_quota_inputs.into(),
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
            private_inputs: proof_of_leadership_quota_inputs.into(),
        }
    }
}
