use groth16::{Field as _, Fr};

use crate::crypto::keys::Ed25519PublicKey;

pub struct Inputs {
    pub session: u64,
    pub core_root: Fr,
    pub pol_ledger_aged: Fr,
    pub pol_epoch_nonce: Fr,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
    pub signing_key: Ed25519PublicKey,
}

#[cfg(test)]
impl Default for Inputs {
    fn default() -> Self {
        use crate::crypto::keys::Ed25519PrivateKey;

        Self {
            core_quota: u64::default(),
            core_root: Fr::ZERO,
            leader_quota: u64::default(),
            pol_epoch_nonce: Fr::ZERO,
            pol_ledger_aged: Fr::ZERO,
            session: u64::default(),
            signing_key: Ed25519PrivateKey::generate().public_key(),
            total_stake: u64::default(),
        }
    }
}
