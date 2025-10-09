use nomos_core::crypto::ZkHash;

use crate::crypto::keys::Ed25519PublicKey;

/// Public inputs for all types of Proof of Quota. Spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#25a261aa09df80ce943dce35dd5403ac>.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub signing_key: Ed25519PublicKey,
    pub session: u64,
    pub core: CoreInputs,
    pub leader: LeaderInputs,
}

#[cfg(test)]
impl Default for Inputs {
    fn default() -> Self {
        Self {
            signing_key: [0; _].try_into().unwrap(),
            session: 1,
            core: CoreInputs::default(),
            leader: LeaderInputs::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
pub struct CoreInputs {
    pub zk_root: ZkHash,
    pub quota: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Default))]
pub struct LeaderInputs {
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub message_quota: u64,
    pub total_stake: u64,
}
