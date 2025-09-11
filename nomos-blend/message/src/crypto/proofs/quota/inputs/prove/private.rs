use nomos_core::crypto::ZkHash;

/// Private inputs for all types of Proof of Quota. Spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81a18576f67b910d34d4>.
#[non_exhaustive]
pub struct Inputs {
    pub key_index: u64,
    pub selector: bool,
    pub proof_type: ProofType,
}

impl Inputs {
    #[must_use]
    pub fn new_proof_of_core_quota_inputs(
        key_index: u64,
        proof_of_core_quota_inputs: ProofOfCoreQuotaInputs,
    ) -> Self {
        Self {
            key_index,
            selector: false,
            proof_type: proof_of_core_quota_inputs.into(),
        }
    }

    #[must_use]
    pub fn new_proof_of_leadership_quota_inputs(
        key_index: u64,
        proof_of_leadership_quota_inputs: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        Self {
            key_index,
            selector: true,
            proof_type: proof_of_leadership_quota_inputs.into(),
        }
    }
}

#[cfg(test)]
impl Default for Inputs {
    fn default() -> Self {
        use groth16::Field as _;

        Self {
            key_index: u64::MIN,
            proof_type: ProofType::CoreQuota(ProofOfCoreQuotaInputs {
                core_path: vec![],
                core_path_selectors: vec![],
                core_sk: ZkHash::ZERO,
            }),
            selector: false,
        }
    }
}

pub enum ProofType {
    CoreQuota(ProofOfCoreQuotaInputs),
    LeadershipQuota(ProofOfLeadershipQuotaInputs),
}

pub struct ProofOfCoreQuotaInputs {
    pub core_sk: ZkHash,
    pub core_path: Vec<ZkHash>,
    pub core_path_selectors: Vec<bool>,
}

impl From<ProofOfCoreQuotaInputs> for ProofType {
    fn from(value: ProofOfCoreQuotaInputs) -> Self {
        Self::CoreQuota(value)
    }
}

pub struct ProofOfLeadershipQuotaInputs {
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

impl From<ProofOfLeadershipQuotaInputs> for ProofType {
    fn from(value: ProofOfLeadershipQuotaInputs) -> Self {
        Self::LeadershipQuota(value)
    }
}
