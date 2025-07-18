mod config;
// The edger is split into two modules:
// - `cryptarchia`: the base functionalities needed by the Cryptarchia consensus
//   algorithm, including a minimal UTxO model.
// - `mantle_ops` : our extensions in the form of Mantle operations, e.g. SDP.
pub mod cryptarchia;
pub mod mantle;

use std::{collections::HashMap, hash::Hash};

pub use config::Config;
use cryptarchia::LedgerState as CryptarchiaLedger;
pub use cryptarchia::{EpochState, UtxoTree};
use cryptarchia_engine::Slot;
use nomos_core::{mantle::Utxo, proofs::leader_proof};
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LedgerError<Id> {
    #[error("Invalid block slot {block:?} for parent slot {parent:?}")]
    InvalidSlot { parent: Slot, block: Slot },
    #[error("Parent block not found: {0:?}")]
    ParentNotFound(Id),
    #[error("Invalid leader proof")]
    InvalidProof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ledger<Id: Eq + Hash> {
    states: HashMap<Id, LedgerState>,
    config: Config,
}

impl<Id> Ledger<Id>
where
    Id: Eq + Hash + Copy,
{
    pub fn new(id: Id, state: LedgerState, config: Config) -> Self {
        Self {
            states: std::iter::once((id, state)).collect(),
            config,
        }
    }

    /// Create a new [`Ledger`] with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn try_update<LeaderProof>(
        &self,
        id: Id,
        parent_id: Id,
        slot: Slot,
        proof: &LeaderProof,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        let parent_state = self
            .states
            .get(&parent_id)
            .ok_or(LedgerError::ParentNotFound(parent_id))?;

        let new_state = parent_state.clone().try_update(slot, proof, &self.config)?;

        let mut states = self.states.clone();
        states.insert(id, new_state);
        Ok(Self {
            states,
            config: self.config,
        })
    }

    pub fn state(&self, id: &Id) -> Option<&LedgerState> {
        self.states.get(id)
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Removes the state stored for the given block id.
    ///
    /// This function must be called only when the states being pruned won't be
    /// needed for any subsequent proof.
    ///
    /// ## Arguments
    ///
    /// The block ID to prune the state for.
    ///
    /// ## Returns
    ///
    /// `true` if the state was successfully removed, `false` otherwise.
    pub fn prune_state_at(&mut self, block: &Id) -> bool {
        self.states.remove(block).is_some()
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LedgerState {
    cryptarchia_ledger: CryptarchiaLedger,
    mantle_ledger: (),
}

impl LedgerState {
    fn try_update<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        self.try_apply_header(slot, proof, config)?
            .try_apply_contents()
    }

    /// Apply header-related changed to the ledger state. These include
    /// leadership and in general any changes that not related to
    /// transactions that should be applied before that.
    fn try_apply_header<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        let cryptarchia_ledger = self
            .cryptarchia_ledger
            .try_apply_header::<LeaderProof, Id>(slot, proof, config)?;
        // If we need to do something for mantle ops/rewards, this would be the place.
        Ok(Self {
            cryptarchia_ledger,
            mantle_ledger: (),
        })
    }

    #[expect(
        clippy::missing_const_for_fn,
        clippy::unnecessary_wraps,
        reason = "placehoder method"
    )]
    fn try_apply_contents<Id>(self) -> Result<Self, LedgerError<Id>> {
        // In the current implementation, we don't have any contents to apply.
        // If we had transactions or other contents, this would be the place to apply
        // them.
        Ok(self)
    }

    pub fn from_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        Self {
            cryptarchia_ledger: CryptarchiaLedger::from_utxos(utxos),
            mantle_ledger: (),
        }
    }

    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.cryptarchia_ledger.slot()
    }

    #[must_use]
    pub const fn epoch_state(&self) -> &EpochState {
        self.cryptarchia_ledger.epoch_state()
    }

    #[must_use]
    pub const fn next_epoch_state(&self) -> &EpochState {
        self.cryptarchia_ledger.next_epoch_state()
    }

    #[must_use]
    pub const fn latest_commitments(&self) -> &UtxoTree {
        self.cryptarchia_ledger.latest_commitments()
    }

    #[must_use]
    pub const fn aged_commitments(&self) -> &UtxoTree {
        self.cryptarchia_ledger.aged_commitments()
    }
}
