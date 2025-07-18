use serde::{Deserialize, Serialize};

pub type RewardsRoot = [u8; 32];
pub type VoucherNullifier = [u8; 32];

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LeaderClaimOp {
    pub rewards_root: RewardsRoot,
    pub voucher_nullifier: VoucherNullifier,
}
