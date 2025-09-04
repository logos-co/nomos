use nomos_core::crypto::ZkHash;

pub struct ProofOfSelectionInputs {
    pub secret_key: ZkHash,
    pub ephemeral_key_index: usize,
    pub session_number: u64,
}
