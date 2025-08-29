use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};

pub type RewardsRoot = [u8; 32];
pub type VoucherNullifier = [u8; 32];

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LeaderClaimOp {
    pub rewards_root: RewardsRoot,
    pub voucher_nullifier: VoucherNullifier,
}

impl LeaderClaimOp {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(&self.rewards_root);
        buff.extend_from_slice(&self.voucher_nullifier);
        buff.freeze()
    }
}
