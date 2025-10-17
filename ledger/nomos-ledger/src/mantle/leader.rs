use std::cmp::Ordering;

use cryptarchia_engine::Epoch;
use mmr::MerkleMountainRange;
use nomos_core::mantle::{
    Value,
    ops::leader_claim::{LeaderClaimOp, RewardsRoot, VoucherCm, VoucherNullifier},
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
    claimable_rewards: Value,
    // Merkle tree of vouchers, vouchers can only be claimed with a delay
    // of one epoch.
    vouchers: MerkleMountainRange<VoucherCm, nomos_core::crypto::ZkHasher>,
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

impl Default for LeaderState {
    fn default() -> Self {
        Self::new()
    }
}

impl LeaderState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: 0.into(),
            claimable_vouchers_root: RewardsRoot::default(),
            n_claimable_vouchers: 0,
            nfs: rpds::HashTrieSetSync::new_sync(),
            claimable_rewards: 0,
            vouchers: MerkleMountainRange::new(),
        }
    }

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
                self.claimable_vouchers_root = self.vouchers.frontier_root().into();
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
            self.claimable_rewards / n_unclaimed_vouchers
        } else {
            0
        };

        let claimable_rewards = self.claimable_rewards - reward_amount;
        Ok((
            Self {
                epoch: self.epoch,
                claimable_vouchers_root: self.claimable_vouchers_root,
                n_claimable_vouchers: self.n_claimable_vouchers,
                nfs,
                claimable_rewards,
                vouchers: self.vouchers.clone(),
            },
            Balance::from(reward_amount),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::fields::AdditiveGroup;
    use groth16::{Field as _, Fr};
    use nomos_core::mantle::TxHash;

    #[test]
    fn test_reward_amounts() {
        let state = LeaderState::new();
        let state = state.try_apply_header(1.into(), Fr::ZERO.into()).unwrap();
        let state = state.try_apply_header(1.into(), Fr::ONE.into()).unwrap();
        let state = state
            .try_apply_header(1.into(), Fr::from(2u64).into())
            .unwrap();
        let state = state
            .try_apply_header(2.into(), Fr::from(3u64).into())
            .unwrap();
        let state = LeaderState {
            claimable_rewards: 300,
            ..state
        };
        let op1 = LeaderClaimOp {
            rewards_root: state.claimable_vouchers_root,
            voucher_nullifier: Fr::ZERO.into(),
            mantle_tx_hash: TxHash::default(),
        };
        let (state, bal) = state.claim(&op1).unwrap();
        assert_eq!(bal, 100);
        assert_eq!(state.claimable_rewards, 200);
        let op2 = LeaderClaimOp {
            rewards_root: state.claimable_vouchers_root,
            voucher_nullifier: Fr::ONE.into(),
            mantle_tx_hash: TxHash::default(),
        };
        let (state, bal) = state.claim(&op2).unwrap();
        assert_eq!(bal, 100);
        assert_eq!(state.claimable_rewards, 100);
        let op3 = LeaderClaimOp {
            rewards_root: state.claimable_vouchers_root,
            voucher_nullifier: Fr::from(2u64).into(),
            mantle_tx_hash: TxHash::default(),
        };
        let (state, bal) = state.claim(&op3).unwrap();
        assert_eq!(bal, 100);
        assert_eq!(state.claimable_rewards, 0);
    }

    #[test]
    fn test_epoch_transition() {
        let state = LeaderState::new();
        let state = state.try_apply_header(1.into(), Fr::ZERO.into()).unwrap();
        assert_eq!(state.epoch, 1.into());
        assert_eq!(state.n_claimable_vouchers, 0);
        let state = state.try_apply_header(2.into(), Fr::ONE.into()).unwrap();
        assert_eq!(state.epoch, 2.into());
        assert_eq!(state.n_claimable_vouchers, 1);
        let state = state
            .try_apply_header(2.into(), Fr::from(2u64).into())
            .unwrap();
        assert_eq!(state.epoch, 2.into());
        assert_eq!(state.n_claimable_vouchers, 1);
        let state = state
            .try_apply_header(3.into(), Fr::from(3u64).into())
            .unwrap();
        assert_eq!(state.epoch, 3.into());
        assert_eq!(state.n_claimable_vouchers, 3);
        let err = state
            .clone()
            .try_apply_header(2.into(), Fr::from(4u64).into())
            .unwrap_err();
        assert_eq!(
            err,
            Error::InvalidEpoch {
                current: 3.into(),
                incoming: 2.into()
            }
        );
        let state = state
            .try_apply_header(4.into(), Fr::from(5u64).into())
            .unwrap();
        assert_eq!(state.epoch, 4.into());
        assert_eq!(state.n_claimable_vouchers, 4);
    }

    #[test]
    fn test_cannot_claim_reward_twice() {
        let state = LeaderState::new();
        let op = LeaderClaimOp {
            voucher_nullifier: Fr::ZERO.into(),
            rewards_root: state.claimable_vouchers_root,
            mantle_tx_hash: TxHash::default(),
        };
        let (state, balance) = state.claim(&op).unwrap();
        assert_eq!(balance, 0);
        let err = state.claim(&op).unwrap_err();
        assert_eq!(err, Error::DuplicatedVoucherNullifier);
    }
}
