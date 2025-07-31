use std::cmp::Ordering;

use cryptarchia_engine::Epoch;
use mmr::MerkleMountainRange;
use nomos_core::mantle::{
    ops::leader_claim::{LeaderClaimOp, RewardsRoot, VoucherCm, VoucherNullifier},
    Value,
};

use crate::Balance;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderState {
    // current epoch
    epoch: Epoch,
    // vouchers that can be claimed in this epoch
    // this is updated once at the start of each epoch from the root of
    // the vouchers merkle tree
    claimable_vouchers_root: RewardsRoot,
    n_claimable_vouchers: u64,
    // nullifiers of vouchers that have been claimed since genesis
    nfs: rpds::HashTrieSetSync<VoucherNullifier>,
    // rewards to be distributed
    // at the start of each epoch this is increased by the amount of rewards
    // that have been collected in the previous epoch.
    // unclaimed rewards are carried over to the next epoch.
    rewards: Value,
    // Merkle tree of vouchers, vouchers can only be claimed with a delay
    // of one epoch.
    vouchers: MerkleMountainRange<VoucherCm, nomos_core::crypto::Hasher>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("voucher nullifier already used")]
    DuplicatedVoucherNullifier,
    #[error("voucher not found")]
    VoucherNotFound,
    #[error("Cannot time travel to the past")]
    InvalidEpoch { current: Epoch, incoming: Epoch },
}

impl LeaderState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: 0.into(),
            claimable_vouchers_root: [0; 32],
            n_claimable_vouchers: 0,
            nfs: rpds::HashTrieSetSync::new_sync(),
            rewards: 0,
            vouchers: MerkleMountainRange::new(),
        }
    }

    #[must_use]
    pub fn try_apply_header(self, epoch: Epoch, voucher_cm: VoucherCm) -> Result<Self, Error> {
        Ok(self.update_epoch_state(epoch)?.add_voucher(voucher_cm))
    }

    fn update_epoch_state(mut self, epoch: Epoch) -> Result<Self, Error> {
        match epoch.cmp(&self.epoch) {
            Ordering::Equal => Ok(self),
            Ordering::Less => Err(Error::InvalidEpoch {
                current: self.epoch,
                incoming: epoch,
            }),
            Ordering::Greater => {
                self.epoch = epoch;
                self.claimable_vouchers_root = self.vouchers.frontier_root();
                self.n_claimable_vouchers = self.vouchers.len() as u64;
                // TODO: increase rewards, what about epoch jumps?
                Ok(self)
            }
        }
    }

    fn add_voucher(self, voucher_cm: VoucherCm) -> Self {
        Self {
            vouchers: self.vouchers.push(voucher_cm),
            ..self
        }
    }

    /// Claim the reward associated with a voucher.
    /// Any cryptographic proof of correct derivation of the voucher nullifier
    /// and membership proof in the merkle tree is expected to happen
    /// outside of this function.
    pub fn claim(&self, op: &LeaderClaimOp) -> Result<(Self, Balance), Error> {
        if self.nfs.contains(&op.voucher_nullifier) {
            return Err(Error::DuplicatedVoucherNullifier);
        }

        if self.claimable_vouchers_root != op.rewards_root {
            return Err(Error::VoucherNotFound);
        }

        let nfs = self.nfs.insert(op.voucher_nullifier);
        let n_unclaimed_vouchers = self
            .n_claimable_vouchers
            .checked_sub(self.nfs.size() as u64)
            .expect("more nullifiers than vouchers");
        let reward_amount = if n_unclaimed_vouchers > 0 {
            self.rewards / n_unclaimed_vouchers
        } else {
            0
        };

        let new_rewards = self.rewards - reward_amount;
        Ok((
            Self {
                epoch: self.epoch,
                claimable_vouchers_root: self.claimable_vouchers_root,
                n_claimable_vouchers: self.n_claimable_vouchers,
                nfs,
                rewards: new_rewards,
                vouchers: self.vouchers.clone(),
            },
            Balance::from(reward_amount),
        ))
    }
}
