use nomos_core::crypto::ZkHash;

pub enum SecretKey {
    Core(ZkHash),
    PoL(u64),
}

pub struct ProofOfSelectionInputs {
    pub secret_key: SecretKey,
    pub ephemeral_key_index: usize,
    pub session_number: u64,
}
