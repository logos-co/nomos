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
use mantle::LedgerState as MantleLedger;
use nomos_core::{
    block::BlockNumber,
    mantle::{gas::GasConstants, AuthenticatedMantleTx, NoteId, Utxo},
    proofs::leader_proof,
};
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LedgerError<Id> {
    #[error("Invalid block slot {block:?} for parent slot {parent:?}")]
    InvalidSlot { parent: Slot, block: Slot },
    #[error("Parent block not found: {0:?}")]
    ParentNotFound(Id),
    #[error("Invalid leader proof")]
    InvalidProof,
    #[error("Invalid note: {0:?}")]
    InvalidNote(NoteId),
    #[error("Insufficient balance")]
    InsufficientBalance,
    #[error("Overflow while calculating balance")]
    Overflow,
    #[error("Zero value note")]
    ZeroValueNote,
    #[error("Mantle error: {0}")]
    Mantle(#[from] mantle::Error),
    #[error("Locked note: {0:?}")]
    LockedNote(NoteId),
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
    pub fn try_update<LeaderProof, Constants>(
        &self,
        id: Id,
        parent_id: Id,
        slot: Slot,
        current_block_number: BlockNumber,
        proof: &LeaderProof,
        txs: impl Iterator<Item = impl AuthenticatedMantleTx>,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
        Constants: GasConstants,
    {
        let parent_state = self
            .states
            .get(&parent_id)
            .ok_or(LedgerError::ParentNotFound(parent_id))?;

        let new_state = parent_state.clone().try_update::<_, _, Constants>(
            slot,
            proof,
            current_block_number,
            txs,
            &self.config,
        )?;

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
    mantle_ledger: MantleLedger,
}

impl LedgerState {
    fn try_update<LeaderProof, Id, Constants>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        current_block_number: BlockNumber,
        txs: impl Iterator<Item = impl AuthenticatedMantleTx>,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
        Constants: GasConstants,
    {
        self.try_apply_header(slot, proof, config)?
            .try_apply_contents::<_, Constants>(current_block_number, config, txs)
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
            mantle_ledger: self.mantle_ledger,
        })
    }

    /// Apply the contents of an update to the ledger state.
    fn try_apply_contents<Id, Constants: GasConstants>(
        mut self,
        current_block_number: BlockNumber,
        config: &Config,
        txs: impl Iterator<Item = impl AuthenticatedMantleTx>,
    ) -> Result<Self, LedgerError<Id>> {
        for tx in txs {
            let _balance;
            (self.cryptarchia_ledger, _balance) = self
                .cryptarchia_ledger
                .try_apply_tx::<_, Constants>(&self.mantle_ledger.locked_notes(), &tx)?;

            self.mantle_ledger = self.mantle_ledger.try_apply_tx::<Constants>(
                current_block_number,
                &config,
                &self.cryptarchia_ledger.latest_commitments(),
                tx,
            )?;
        }
        Ok(self)
    }

    pub fn from_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        Self {
            cryptarchia_ledger: CryptarchiaLedger::from_utxos(utxos),
            mantle_ledger: MantleLedger::default(),
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

#[cfg(test)]
mod tests {
    use cryptarchia::tests::{config, generate_proof, utxo};
    use groth16::Fr;
    use nomos_core::{
        mantle::{
            gas::MainnetGasConstants, keys::PublicKey, ledger::Tx as LedgerTx, MantleTx, Note,
            SignedMantleTx, Transaction as _,
        },
        proofs::zksig::DummyZkSignature,
    };
    use num_bigint::BigUint;

    use super::*;

    type HeaderId = [u8; 32];

    fn create_tx(inputs: Vec<NoteId>, outputs: Vec<Note>, pks: Vec<Fr>) -> SignedMantleTx {
        let ledger_tx = LedgerTx::new(inputs, outputs);
        let mantle_tx = MantleTx {
            ops: vec![],
            ledger_tx,
            execution_gas_price: 1,
            storage_gas_price: 1,
        };
        SignedMantleTx {
            ops_profs: vec![],
            ledger_tx_proof: DummyZkSignature::prove(
                nomos_core::proofs::zksig::ZkSignaturePublic {
                    pks,
                    msg_hash: mantle_tx.hash().into(),
                },
            ),
            mantle_tx,
        }
    }

    pub fn create_test_ledger() -> (Ledger<HeaderId>, HeaderId, Utxo) {
        let utxo = utxo();
        let genesis_state = LedgerState::from_utxos([utxo]);
        let ledger = Ledger::new([0; 32], genesis_state, config());
        (ledger, [0; 32], utxo)
    }

    #[test]
    fn test_ledger_creation() {
        let (ledger, genesis_id, utxo) = create_test_ledger();

        let state = ledger.state(&genesis_id).unwrap();
        assert!(state.latest_commitments().contains(&utxo.id()));
        assert_eq!(state.slot(), 0.into());
    }

    #[test]
    fn test_ledger_try_update_with_transaction() {
        let (ledger, genesis_id, utxo) = create_test_ledger();

        let output_note = Note::new(1, PublicKey::new(BigUint::from(1u8).into()));
        let pk = BigUint::from(0u8).into();
        let tx = create_tx(vec![utxo.id()], vec![output_note], vec![pk]);

        // Create a dummy proof (using same structure as in cryptarchia tests)

        let proof = generate_proof(
            &ledger.state(&genesis_id).unwrap().cryptarchia_ledger,
            &utxo,
            Slot::from(1u64),
            &ledger.config,
        );

        let new_id = [1; 32];
        let new_ledger = ledger
            .try_update::<_, MainnetGasConstants>(
                new_id,
                genesis_id,
                Slot::from(1u64),
                1,
                &proof,
                std::iter::once(&tx),
            )
            .unwrap();

        // Verify the transaction was applied
        let new_state = new_ledger.state(&new_id).unwrap();
        assert!(!new_state.latest_commitments().contains(&utxo.id()));

        // Verify output was created
        let output_utxo = tx.mantle_tx.ledger_tx.utxo_by_index(0).unwrap();
        assert!(new_state.latest_commitments().contains(&output_utxo.id()));
    }
}
