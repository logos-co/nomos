pub mod error;

use std::collections::{BTreeMap, HashSet};

pub use error::WalletError;
use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, NoteId, Utxo, Value, keys::PublicKey, ledger::Tx as LedgerTx},
};
use nomos_ledger::LedgerState;

pub struct WalletBlock {
    pub id: HeaderId,
    pub parent: HeaderId,
    pub ledger_txs: Vec<LedgerTx>,
}

impl<Tx: AuthenticatedMantleTx> From<Block<Tx>> for WalletBlock {
    fn from(block: Block<Tx>) -> Self {
        Self {
            id: block.header().id(),
            parent: block.header().parent(),
            ledger_txs: block
                .transactions()
                .map(|auth_tx| auth_tx.mantle_tx().ledger_tx.clone())
                .collect(),
        }
    }
}

#[derive(Clone)]
pub struct WalletState {
    pub utxos: rpds::HashTrieMapSync<NoteId, Utxo>,
    pub pk_index: rpds::HashTrieMapSync<PublicKey, rpds::HashTrieSetSync<NoteId>>,
}

impl WalletState {
    pub fn from_ledger(known_keys: &HashSet<PublicKey>, ledger: &LedgerState) -> Self {
        let mut utxos = rpds::HashTrieMapSync::new_sync();
        let mut pk_index = rpds::HashTrieMapSync::new_sync();

        for (_, (utxo, _)) in ledger.latest_utxos().utxos().iter() {
            if known_keys.contains(&utxo.note.pk) {
                let note_id = utxo.id();
                utxos = utxos.insert(note_id, *utxo);

                let note_set = pk_index
                    .get(&utxo.note.pk)
                    .cloned()
                    .unwrap_or_else(rpds::HashTrieSetSync::new_sync)
                    .insert(note_id);
                pk_index = pk_index.insert(utxo.note.pk, note_set);
            }
        }

        Self { utxos, pk_index }
    }

    pub fn utxos_for_amount(
        &self,
        amount: Value,
        pks: impl IntoIterator<Item = PublicKey>,
    ) -> Option<Vec<Utxo>> {
        let mut utxos: Vec<Utxo> = pks
            .into_iter()
            .filter_map(|pk| self.pk_index.get(&pk))
            .flatten()
            .map(|id| self.utxos[id])
            .collect();

        // we want to consume small valued notes first to keep our wallet tidy
        utxos.sort_by_key(|utxo| utxo.note.value);

        let mut selected_utxos = Vec::new();
        let mut selected_amount = 0;

        for utxo in utxos {
            selected_utxos.push(utxo);
            selected_amount += utxo.note.value;
            if selected_amount >= amount {
                break;
            }
        }

        if selected_amount < amount {
            None
        } else {
            Some(Self::remove_redundant_utxos(amount, selected_utxos))
        }
    }

    #[must_use]
    pub fn balance(&self, pk: PublicKey) -> Option<Value> {
        let balance = self
            .pk_index
            .get(&pk)?
            .iter()
            .map(|id| self.utxos[id].note.value)
            .sum();

        Some(balance)
    }

    #[must_use]
    pub fn apply_block(&self, known_keys: &HashSet<PublicKey>, block: &WalletBlock) -> Self {
        let mut utxos = self.utxos.clone();
        let mut pk_index = self.pk_index.clone();

        // Process each transaction in the block
        for ledger_tx in &block.ledger_txs {
            // Remove spent UTXOs (inputs)
            for spent_id in &ledger_tx.inputs {
                if let Some(utxo) = utxos.get(spent_id) {
                    let pk = utxo.note.pk;
                    utxos = utxos.remove(spent_id);

                    if let Some(note_set) = pk_index.get(&pk) {
                        let updated_set = note_set.remove(spent_id);
                        if updated_set.is_empty() {
                            pk_index = pk_index.remove(&pk);
                        } else {
                            pk_index = pk_index.insert(pk, updated_set);
                        }
                    }
                }
            }

            // Add new UTXOs (outputs) - only if they belong to our known keys
            for utxo in ledger_tx.utxos() {
                if known_keys.contains(&utxo.note.pk) {
                    let note_id = utxo.id();
                    utxos = utxos.insert(note_id, utxo);

                    let note_set = pk_index
                        .get(&utxo.note.pk)
                        .cloned()
                        .unwrap_or_else(rpds::HashTrieSetSync::new_sync)
                        .insert(note_id);
                    pk_index = pk_index.insert(utxo.note.pk, note_set);
                }
            }
        }

        Self { utxos, pk_index }
    }

    /// Removes Utxos that do not contribute to meeting the `amount` threshold.
    ///
    /// As an example, suppose we hold notes valued [3 NMO, 4 NMO] and we asked
    /// for 4 NMO then, since we sort the notes by value, we would have
    /// first added the 3 NMO note and then then 4 NMO note to the selected
    /// utxos list.
    ///
    /// The 4 NMO note alone would have satisfied the request, the 3 NMO note is
    /// redundant and would be returned as change in a transaction.
    ///
    /// To resolve this, we remove as many of the smallest notes as we can while
    /// still keep us above the requested amount.
    fn remove_redundant_utxos(amount: Value, mut sorted_utxos: Vec<Utxo>) -> Vec<Utxo> {
        debug_assert!(sorted_utxos.is_sorted_by_key(|utxo| utxo.note.value));

        let mut skip_count = 0;
        let mut temp_amount: Value = sorted_utxos.iter().map(|u| u.note.value).sum();

        for utxo in &sorted_utxos {
            if temp_amount - utxo.note.value >= amount {
                temp_amount -= utxo.note.value;
                skip_count += 1;
            } else {
                break;
            }
        }

        sorted_utxos.drain(..skip_count);

        sorted_utxos
    }
}

#[derive(Clone)]
pub struct Wallet {
    known_keys: HashSet<PublicKey>,
    wallet_states: BTreeMap<HeaderId, WalletState>,
}

impl Wallet {
    pub fn from_lib(
        known_keys: impl IntoIterator<Item = PublicKey>,
        lib: HeaderId,
        ledger: &LedgerState,
    ) -> Self {
        let known_keys: HashSet<PublicKey> = known_keys.into_iter().collect();
        let wallet_state = WalletState::from_ledger(&known_keys, ledger);

        Self {
            known_keys,
            wallet_states: [(lib, wallet_state)].into(),
        }
    }

    #[must_use]
    pub const fn known_keys(&self) -> &HashSet<PublicKey> {
        &self.known_keys
    }

    #[must_use]
    pub fn has_processed_block(&self, block_id: HeaderId) -> bool {
        self.wallet_states.contains_key(&block_id)
    }

    pub fn apply_block(&mut self, block: &WalletBlock) -> Result<(), WalletError> {
        if self.wallet_states.contains_key(&block.id) {
            // Already processed this block
            return Ok(());
        }

        let block_wallet_state = self
            .wallet_state_at(block.parent)?
            .apply_block(&self.known_keys, block);
        self.wallet_states.insert(block.id, block_wallet_state);
        Ok(())
    }

    pub fn balance(&self, tip: HeaderId, pk: PublicKey) -> Result<Option<Value>, WalletError> {
        Ok(self.wallet_state_at(tip)?.balance(pk))
    }

    pub fn utxos_for_amount(
        &self,
        tip: HeaderId,
        amount: Value,
        pks: impl IntoIterator<Item = PublicKey>,
    ) -> Result<Option<Vec<Utxo>>, WalletError> {
        Ok(self.wallet_state_at(tip)?.utxos_for_amount(amount, pks))
    }

    pub fn wallet_state_at(&self, tip: HeaderId) -> Result<WalletState, WalletError> {
        self.wallet_states
            .get(&tip)
            .cloned()
            .ok_or(WalletError::UnknownBlock)
    }

    /// Prune wallet states for blocks that have been pruned from the chain.
    ///
    /// This removes wallet states for blocks that are no longer part of the
    /// chain after LIB advancement. Both stale blocks (from abandoned
    /// forks) and immutable blocks (before the new LIB) are removed.
    pub fn prune_states(&mut self, pruned_blocks: impl IntoIterator<Item = HeaderId>) {
        let mut removed_count = 0;

        for block_id in pruned_blocks {
            if self.wallet_states.remove(&block_id).is_some() {
                removed_count += 1;
            }
        }

        if removed_count > 0 {
            tracing::debug!(
                removed_states = removed_count,
                remaining_states = self.wallet_states.len(),
                "Pruned wallet states for pruned blocks"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::mantle::{Note, TxHash};
    use num_bigint::BigUint;

    use super::*;

    fn pk(v: u64) -> PublicKey {
        PublicKey::from(BigUint::from(v))
    }

    fn tx_hash(v: u64) -> TxHash {
        TxHash::from(BigUint::from(v))
    }

    #[test]
    fn test_initialization() {
        let alice = pk(1);
        let bob = pk(2);

        let genesis = HeaderId::from([0; 32]);

        let ledger = LedgerState::from_utxos([
            Utxo::new(tx_hash(0), 0, Note::new(100, alice)),
            Utxo::new(tx_hash(0), 1, Note::new(20, bob)),
            Utxo::new(tx_hash(0), 2, Note::new(4, alice)),
        ]);

        let wallet = Wallet::from_lib([], genesis, &ledger);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::from_lib([alice], genesis, &ledger);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::from_lib([bob], genesis, &ledger);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));

        let wallet = Wallet::from_lib([alice, bob], genesis, &ledger);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));
    }

    #[test]
    fn test_sync() {
        let alice = pk(1);
        let bob = pk(2);

        let genesis = HeaderId::from([0; 32]);

        let genesis_ledger = LedgerState::from_utxos([]);

        let mut wallet = Wallet::from_lib([alice, bob], genesis, &genesis_ledger);

        // Block 1
        // - alice is minted 104 NMO in two notes (100 NMO and 4 NMO)
        let tx1 = LedgerTx {
            inputs: vec![],
            outputs: vec![Note::new(100, alice), Note::new(4, alice)],
        };

        let block_1 = WalletBlock {
            id: HeaderId::from([1; 32]),
            parent: genesis,
            ledger_txs: vec![tx1.clone()],
        };

        wallet.apply_block(&block_1).unwrap();

        // Block 2
        //  - alice spends 100 NMO utxo, sending 20 NMO to bob and 80 to herself
        let utxos_100 = wallet
            .utxos_for_amount(block_1.id, 100, [alice])
            .unwrap()
            .unwrap();

        assert_eq!(utxos_100, vec![tx1.utxo_by_index(0).unwrap()]);

        let block_2 = WalletBlock {
            id: HeaderId::from([2; 32]),
            parent: block_1.id,
            ledger_txs: vec![LedgerTx {
                inputs: utxos_100.iter().map(Utxo::id).collect(),
                outputs: vec![Note::new(20, bob), Note::new(80, alice)],
            }],
        };
        wallet.apply_block(&block_2).unwrap();

        // Query the balance of for each pk at different points in the blockchain
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        assert_eq!(wallet.balance(block_1.id, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(block_1.id, bob).unwrap(), None);

        assert_eq!(wallet.balance(block_2.id, alice).unwrap(), Some(84));
        assert_eq!(wallet.balance(block_2.id, bob).unwrap(), Some(20));
    }

    #[test]
    fn test_utxos_for_amount() {
        let alice_1 = pk(1);
        let alice_2 = pk(2);
        let bob = pk(3);

        let ledger = LedgerState::from_utxos([
            Utxo::new(tx_hash(0), 0, Note::new(4, alice_1)),
            Utxo::new(tx_hash(0), 1, Note::new(3, alice_2)),
            Utxo::new(tx_hash(0), 2, Note::new(5, alice_2)),
            Utxo::new(tx_hash(0), 3, Note::new(10, alice_2)),
            Utxo::new(tx_hash(0), 4, Note::new(20, bob)),
        ]);

        let genesis = HeaderId::from([0u8; 32]);

        let wallet = Wallet::from_lib([alice_1, alice_2, bob], genesis, &ledger);

        // requesting 2 NMO from alices keys
        assert_eq!(
            wallet
                .utxos_for_amount(genesis, 2, [alice_1, alice_2])
                .unwrap(),
            Some(vec![Utxo::new(tx_hash(0), 1, Note::new(3, alice_2))])
        );

        // requesting 3 NMO from alices keys
        assert_eq!(
            wallet
                .utxos_for_amount(genesis, 3, [alice_1, alice_2])
                .unwrap(),
            Some(vec![Utxo::new(tx_hash(0), 1, Note::new(3, alice_2))])
        );

        // requesting 4 NMO from alices keys
        assert_eq!(
            wallet
                .utxos_for_amount(genesis, 4, [alice_1, alice_2])
                .unwrap(),
            Some(vec![Utxo::new(tx_hash(0), 0, Note::new(4, alice_1))])
        );

        // requesting 5 NMO from alices keys
        // returns 2 notes despite a note of exactly 5 NMO available to alice
        assert_eq!(
            wallet
                .utxos_for_amount(genesis, 5, [alice_1, alice_2])
                .unwrap(),
            Some(vec![
                Utxo::new(tx_hash(0), 1, Note::new(3, alice_2)),
                Utxo::new(tx_hash(0), 0, Note::new(4, alice_1)),
            ])
        );
    }
}
