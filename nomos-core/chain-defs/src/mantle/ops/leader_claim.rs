use bytes::{Bytes, BytesMut};
use groth16::serde::serde_fr;
use poseidon2::{Fr, ZkHash};
use serde::{Deserialize, Serialize};

use crate::mantle::TxHash;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
pub struct RewardsRoot(#[serde(with = "serde_fr")] ZkHash);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct VoucherNullifier(#[serde(with = "serde_fr")] ZkHash);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
pub struct VoucherCm(#[serde(with = "serde_fr")] ZkHash);

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LeaderClaimOp {
    pub rewards_root: RewardsRoot,
    pub voucher_nullifier: VoucherNullifier,
    pub mantle_tx_hash: TxHash,
}

impl LeaderClaimOp {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        // TODO: revisit after Fr to bytes standardization
        buff.extend(
            self.rewards_root
                .0
                 .0
                 .0
                .iter()
                .copied()
                .flat_map(u64::to_le_bytes),
        );
        buff.extend(
            self.voucher_nullifier
                .0
                 .0
                 .0
                .iter()
                .copied()
                .flat_map(u64::to_le_bytes),
        );
        buff.extend(
            self.mantle_tx_hash
                .0
                 .0
                 .0
                .iter()
                .copied()
                .flat_map(u64::to_le_bytes),
        );
        buff.freeze()
    }
}

impl AsRef<Fr> for VoucherCm {
    fn as_ref(&self) -> &Fr {
        &self.0
    }
}

impl From<Fr> for VoucherCm {
    fn from(value: Fr) -> Self {
        Self(value)
    }
}

impl From<Fr> for RewardsRoot {
    fn from(value: Fr) -> Self {
        Self(value)
    }
}

impl From<Fr> for VoucherNullifier {
    fn from(value: Fr) -> Self {
        Self(value)
    }
}

impl From<RewardsRoot> for Fr {
    fn from(value: RewardsRoot) -> Self {
        value.0
    }
}

impl From<VoucherNullifier> for Fr {
    fn from(value: VoucherNullifier) -> Self {
        value.0
    }
}

impl From<VoucherCm> for Fr {
    fn from(value: VoucherCm) -> Self {
        value.0
    }
}
