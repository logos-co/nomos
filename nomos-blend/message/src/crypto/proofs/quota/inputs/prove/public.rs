use nomos_core::crypto::ZkHash;

use crate::crypto::keys::Ed25519PublicKey;

/// Public inputs for all types of Proof of Quota. Spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#25a261aa09df80ce943dce35dd5403ac>.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub session: u64,
    pub core_root: ZkHash,
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
    pub signing_key: Ed25519PublicKey,
}

#[cfg(test)]
impl Default for Inputs {
    fn default() -> Self {
        use groth16::Field as _;

        Self {
            core_quota: u64::default(),
            core_root: ZkHash::ZERO,
            leader_quota: u64::default(),
            pol_epoch_nonce: ZkHash::ZERO,
            pol_ledger_aged: ZkHash::ZERO,
            session: u64::default(),
            signing_key: [0; _].try_into().unwrap(),
            total_stake: 1,
        }
    }
}
