use std::collections::HashSet;

use cryptarchia_engine::{Boostrapping, Branch, Branches, ForkDivergenceInfo, Online, Slot};
use nomos_core::header::{Header, HeaderId};
use nomos_ledger::LedgerState;

use crate::{Cryptarchia, Error};

/// A wrapper around `Cryptarchia<CryptarchiaState>`.
/// `CryptarchiaState` is determined/switched in runtime.
pub enum CryptarchiaWrapper {
    Bootstrapping(Cryptarchia<Boostrapping>),
    Online(Cryptarchia<Online>),
}

impl CryptarchiaWrapper {
    pub const fn tip(&self) -> HeaderId {
        match self {
            Self::Bootstrapping(c) => c.tip(),
            Self::Online(c) => c.tip(),
        }
    }

    pub const fn tip_branch(&self) -> &Branch<HeaderId> {
        match self {
            Self::Bootstrapping(c) => c.consensus.tip_branch(),
            Self::Online(c) => c.consensus.tip_branch(),
        }
    }

    pub fn tip_state(&self) -> &LedgerState {
        match self {
            Self::Bootstrapping(c) => c.tip_state(),
            Self::Online(c) => c.tip_state(),
        }
    }

    pub const fn lib(&self) -> HeaderId {
        match self {
            Self::Bootstrapping(c) => c.lib(),
            Self::Online(c) => c.lib(),
        }
    }

    pub fn lib_branch(&self) -> &Branch<HeaderId> {
        match self {
            Self::Bootstrapping(c) => c.consensus.lib_branch(),
            Self::Online(c) => c.consensus.lib_branch(),
        }
    }

    pub fn has_block(&self, id: &HeaderId) -> bool {
        match self {
            Self::Bootstrapping(c) => c.has_block(id),
            Self::Online(c) => c.has_block(id),
        }
    }

    pub const fn branches(&self) -> &Branches<HeaderId> {
        match self {
            Self::Bootstrapping(c) => c.consensus.branches(),
            Self::Online(c) => c.consensus.branches(),
        }
    }

    pub fn state(&self, id: &HeaderId) -> Option<&LedgerState> {
        match self {
            Self::Bootstrapping(c) => c.ledger.state(id),
            Self::Online(c) => c.ledger.state(id),
        }
    }

    pub fn try_apply_header(&self, header: &Header) -> Result<Self, Error> {
        match self {
            Self::Bootstrapping(c) => Ok(Self::Bootstrapping(c.try_apply_header(header)?)),
            Self::Online(c) => Ok(Self::Online(c.try_apply_header(header)?)),
        }
    }

    pub fn epoch_state_for_slot(&self, slot: Slot) -> Option<&nomos_ledger::EpochState> {
        match self {
            Self::Bootstrapping(c) => c.epoch_state_for_slot(slot),
            Self::Online(c) => c.epoch_state_for_slot(slot),
        }
    }

    pub fn prune_old_forks(&mut self) -> HashSet<HeaderId> {
        match self {
            Self::Bootstrapping(c) => c.prune_old_forks().collect(),
            Self::Online(c) => c.prune_old_forks().collect(),
        }
    }

    pub fn prunable_forks(&self, max_div_depth: u64) -> Vec<ForkDivergenceInfo<HeaderId>> {
        match self {
            Self::Bootstrapping(c) => c.consensus.prunable_forks(max_div_depth).collect(),
            Self::Online(c) => c.consensus.prunable_forks(max_div_depth).collect(),
        }
    }
}
