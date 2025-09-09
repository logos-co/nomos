use groth16::Fr;

pub mod decapsulated;
pub mod encapsulated;
pub mod unwrapped;

#[cfg(test)]
mod tests;

pub struct ProofOfQuotaDecapsulationInputs {
    pub session: u64,
    pub core_root: Fr,
    pub pol_ledger_aged: Fr,
    pub pol_epoch_nonce: Fr,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
}

pub struct ProofOfQuotaVerificationInputs {
    pub decapsulation_inputs: ProofOfQuotaDecapsulationInputs,
    pub key_nullifier: Fr,
}

impl ProofOfQuotaVerificationInputs {
    #[must_use] pub const fn from_depapsulation_inputs_and_nullifier(
        decapsulation_inputs: ProofOfQuotaDecapsulationInputs,
        key_nullifier: Fr,
    ) -> Self {
        Self {
            decapsulation_inputs,
            key_nullifier,
        }
    }
}
