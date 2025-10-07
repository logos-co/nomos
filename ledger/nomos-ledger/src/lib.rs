mod config;
// The edger is split into two modules:
// - `cryptarchia`: the base functionalities needed by the Cryptarchia consensus
//   algorithm, including a minimal UTxO model.
// - `mantle_ops` : our extensions in the form of Mantle operations, e.g. SDP.
pub mod cryptarchia;
pub mod mantle;

use std::{cmp::Ordering, collections::HashMap, hash::Hash};

pub use config::Config;
use cryptarchia::LedgerState as CryptarchiaLedger;
pub use cryptarchia::{EpochState, UtxoTree};
use cryptarchia_engine::Slot;
use groth16::{Field as _, Fr};
use mantle::{LedgerState as MantleLedger, sdp::Sessions};
use nomos_core::{
    block::BlockNumber,
    mantle::{
        AuthenticatedMantleTx, GenesisTx, NoteId, Utxo, gas::GasConstants,
        ops::leader_claim::VoucherCm,
    },
    proofs::leader_proof,
    sdp::{ProviderId, ProviderInfo, ServiceType},
};
use thiserror::Error;

// While individual notes are constrained to be `u64`, intermediate calculations
// may overflow, so we use `i128` to avoid that and to easily represent negative
// balances which may arise in special circumstances (e.g. rewards calculation).
pub type Balance = i128;

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
    #[error("Unbalanced transaction, balance does not match fees")]
    UnbalancedTransaction,
    #[error("Overflow while calculating balance")]
    Overflow,
    #[error("Zero value note")]
    ZeroValueNote,
    #[error("Mantle error: {0}")]
    Mantle(#[from] mantle::Error),
    #[error("Locked note: {0:?}")]
    LockedNote(NoteId),
    #[error("Input note in genesis block: {0:?}")]
    InputInGenesis(NoteId),
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
        proof: &LeaderProof,
        voucher: VoucherCm,
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
            voucher,
            txs,
            &self.config,
        )?;

        let mut states = self.states.clone();
        states.insert(id, new_state);
        Ok(Self {
            states,
            config: self.config.clone(),
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
    block_number: BlockNumber,
    cryptarchia_ledger: CryptarchiaLedger,
    mantle_ledger: MantleLedger,
}

impl LedgerState {
    fn try_update<LeaderProof, Id, Constants>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        voucher: VoucherCm,
        txs: impl Iterator<Item = impl AuthenticatedMantleTx>,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
        Constants: GasConstants,
    {
        self.try_apply_header(slot, proof, voucher, config)?
            .try_apply_contents::<_, Constants>(config, txs)
    }

    /// Apply header-related changed to the ledger state. These include
    /// leadership and in general any changes that not related to
    /// transactions that should be applied before that.
    fn try_apply_header<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        voucher: VoucherCm,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        let cryptarchia_ledger = self
            .cryptarchia_ledger
            .try_apply_header::<LeaderProof, Id>(slot, proof, config)?;
        let mantle_ledger = self
            .mantle_ledger
            .try_apply_header(config.epoch(slot), voucher)?;
        Ok(Self {
            block_number: self
                .block_number
                .checked_add(1)
                .expect("Nomos lived long and prospered"),
            cryptarchia_ledger,
            mantle_ledger,
        })
    }

    /// Apply the contents of an update to the ledger state.
    fn try_apply_contents<Id, Constants: GasConstants>(
        mut self,
        config: &Config,
        txs: impl Iterator<Item = impl AuthenticatedMantleTx>,
    ) -> Result<Self, LedgerError<Id>> {
        for tx in txs {
            let balance;
            (self.cryptarchia_ledger, balance) = self
                .cryptarchia_ledger
                .try_apply_tx::<_, Constants>(self.mantle_ledger.locked_notes(), &tx)?;
            let additional_balance;

            (self.mantle_ledger, additional_balance) =
                self.mantle_ledger.try_apply_tx::<Constants>(
                    self.block_number,
                    config,
                    self.cryptarchia_ledger.latest_commitments(),
                    &tx,
                )?;

            let total_balance = balance
                .checked_add(additional_balance)
                .ok_or(LedgerError::Overflow)?;
            match total_balance.cmp(&tx.gas_cost::<Constants>().into()) {
                Ordering::Less => return Err(LedgerError::InsufficientBalance),
                Ordering::Greater => return Err(LedgerError::UnbalancedTransaction),
                Ordering::Equal => {} // OK!
            }
        }
        self.mantle_ledger = self
            .mantle_ledger
            .try_update_membership(self.block_number, config)?;
        Ok(self)
    }

    pub fn from_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        Self {
            block_number: 0,
            cryptarchia_ledger: CryptarchiaLedger::from_utxos(utxos, Fr::ZERO),
            mantle_ledger: MantleLedger::default(),
        }
    }

    pub fn from_genesis_tx<Id, Constants: GasConstants>(
        tx: impl GenesisTx,
        config: &Config,
        epoch_nonce: Fr,
    ) -> Result<Self, LedgerError<Id>> {
        let cryptarchia_ledger = CryptarchiaLedger::from_genesis_tx(&tx, epoch_nonce)?;
        let mantle_ledger = MantleLedger::from_genesis_tx::<Constants>(
            tx,
            config,
            cryptarchia_ledger.latest_commitments(),
        )?;
        Ok(Self {
            block_number: 0,
            cryptarchia_ledger,
            mantle_ledger,
        })
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

    #[must_use]
    pub const fn active_sessions(&self) -> &Sessions {
        self.mantle_ledger.active_sessions()
    }

    #[must_use]
    pub fn active_session_providers(
        &self,
        service_type: ServiceType,
    ) -> Option<HashMap<ProviderId, ProviderInfo>> {
        self.mantle_ledger.active_session_providers(&service_type)
    }
}

#[cfg(test)]
mod tests {
    use cryptarchia::tests::{config, generate_proof, utxo};
    use nomos_core::{
        mantle::{
            GasCost as _, MantleTx, Note, SignedMantleTx, Transaction as _,
            gas::MainnetGasConstants, keys::PublicKey, ledger::Tx as LedgerTx,
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
            ops_proofs: vec![],
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
        let mut output_note = Note::new(1, PublicKey::new(BigUint::from(1u8).into()));
        let pk = BigUint::from(0u8).into();
        // determine fees
        let tx = create_tx(vec![utxo.id()], vec![output_note], vec![pk]);
        let fees = tx.gas_cost::<MainnetGasConstants>();
        output_note.value = utxo.note.value - fees;
        let tx = create_tx(vec![utxo.id()], vec![output_note], vec![pk]);

        // Create a dummy proof (using same structure as in cryptarchia tests)

        let proof = generate_proof(
            &ledger.state(&genesis_id).unwrap().cryptarchia_ledger,
            &utxo,
            Slot::from(1u64),
        );

        let new_id = [1; 32];
        let new_ledger = ledger
            .try_update::<_, MainnetGasConstants>(
                new_id,
                genesis_id,
                Slot::from(1u64),
                &proof,
                VoucherCm::default(),
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
