use serde::{Deserialize, Serialize};

use crate::gas::{Gas, GasConstants, GasPrice};

pub type RewardsRoot = [u8; 32];
pub type VoucherNullifier = [u8; 32];

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LeaderClaimOp {
    pub rewards_root: RewardsRoot,
    pub voucher_nullifier: VoucherNullifier,
}

impl GasPrice for LeaderClaimOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        Constants::CLAIM_BASE_GAS
    }
}
