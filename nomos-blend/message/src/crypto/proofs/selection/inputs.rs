use groth16::Fr;

pub enum SecretKey {
    Core(Fr),
    PoL(Fr),
}

pub struct ProofOfSelectionInputs {
    pub secret_key: SecretKey,
    pub ephemeral_key_index: u64,
    pub session_number: u64,
}
