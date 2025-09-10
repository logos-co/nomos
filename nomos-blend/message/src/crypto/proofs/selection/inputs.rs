use groth16::Fr;

pub struct VerifyInputs {
    pub expected_index: u64,
    pub total_count: u64,
    pub key_nullifier: Fr,
}
