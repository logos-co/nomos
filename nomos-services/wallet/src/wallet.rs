use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{keys::PublicKey, AuthenticatedMantleTx, NoteId, Utxo, Value},
};
use nomos_ledger::LedgerState;

use crate::{Result, WalletError};

struct WalletState {
    utxos: BTreeMap<NoteId, Utxo>,
    pk_index: HashMap<PublicKey, BTreeSet<NoteId>>,
}

impl WalletState {
    pub fn from_ledger(known_keys: &HashSet<PublicKey>, ledger: LedgerState) -> Self {
        let utxos = BTreeMap::from_iter(
            ledger
                .latest_commitments()
                .utxos()
                .iter()
                .map(|(_, (utxo, _))| *utxo)
                .filter(|utxo| known_keys.contains(&utxo.note.pk))
                .map(|utxo| (utxo.id(), utxo)),
        );

        let mut pk_index: HashMap<PublicKey, BTreeSet<NoteId>> = HashMap::new();

        for (id, utxo) in &utxos {
            pk_index.entry(utxo.note.pk).or_default().insert(*id);
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
            .flat_map(|pk| self.pk_index.get(&pk))
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
            // We may have smaller notes that are redundant.
            //
            // e.g. Suppose we hold notes valued [3 NMO, 4 NMO] and we asked for 4 NMO
            //      then, since we sort the notes by value, we would have first
            //      added the 3 NMO note and then then 4 NMO note to the selected utxos list.
            //
            //      The 4 NMO note alone would have satisfied the request, the 3 NMO note is
            //      redundant and would be returned as change in a transaction.
            //
            // To resolve this, we remove as many of the smallest notes as we can while still
            // keep us above the requested amount.

            // Remove redundant small notes from the beginning while maintaining enough value
            let mut skip_count = 0;
            let mut temp_amount = selected_amount;

            for utxo in &selected_utxos {
                if temp_amount - utxo.note.value >= amount {
                    temp_amount -= utxo.note.value;
                    skip_count += 1;
                } else {
                    break;
                }
            }

            // Remove the redundant notes from the beginning
            selected_utxos.drain(..skip_count);

            Some(selected_utxos)
        }
    }

    pub fn balance(&self, pk: PublicKey) -> Option<Value> {
        let balance = self
            .pk_index
            .get(&pk)?
            .iter()
            .map(|id| self.utxos[id].note.value)
            .sum();

        Some(balance)
    }

    pub fn apply_block<T: AuthenticatedMantleTx + Clone + Eq>(
        &self,
        known_keys: &HashSet<PublicKey>,
        block: Block<T, ()>,
    ) -> Self {
        let mut utxos = self.utxos.clone();
        let mut pk_index = self.pk_index.clone();

        // Process each transaction in the block
        for authenticated_tx in block.transactions() {
            let ledger_tx = &authenticated_tx.mantle_tx().ledger_tx;

            // Remove spent UTXOs (inputs)
            for spent_id in &ledger_tx.inputs {
                if let Some(utxo) = utxos.remove(spent_id) {
                    let note_ids = pk_index
                        .get_mut(&utxo.note.pk)
                        .expect("pk_index missing entry for utxo");

                    note_ids.remove(spent_id);

                    if note_ids.is_empty() {
                        pk_index.remove(&utxo.note.pk);
                    }
                }
            }

            // Add new UTXOs (outputs) - only if they belong to our known keys
            for utxo in ledger_tx.utxos() {
                if known_keys.contains(&utxo.note.pk) {
                    let note_id = utxo.id();
                    utxos.insert(note_id, utxo);
                    pk_index.entry(utxo.note.pk).or_default().insert(note_id);
                }
            }
        }

        Self { utxos, pk_index }
    }
}

struct Wallet {
    known_keys: HashSet<PublicKey>,
    wallet_states: BTreeMap<HeaderId, WalletState>,
}

impl Wallet {
    pub fn from_lib(
        known_keys: impl IntoIterator<Item = PublicKey>,
        lib: HeaderId,
        ledger: LedgerState,
    ) -> Self {
        let known_keys: HashSet<PublicKey> = known_keys.into_iter().collect();

        let wallet_state = WalletState::from_ledger(&known_keys, ledger);

        Self {
            known_keys: known_keys.into_iter().collect(),
            wallet_states: [(lib, wallet_state)].into(),
        }
    }

    pub fn apply_block<T: AuthenticatedMantleTx + Clone + Eq>(
        &mut self,
        parent: HeaderId,
        block: Block<T, ()>,
    ) -> Result<()> {
        let block_id = block.header().id();
        let block_wallet_state = self
            .wallet_state_at(parent)?
            .apply_block(&self.known_keys, block);
        self.wallet_states.insert(block_id, block_wallet_state);
        Ok(())
    }

    pub fn apply_ledger(&mut self, block: HeaderId, ledger: LedgerState) {
        // TODO: remove this function in favor of `Wallet::apply_block`
        let wallet_state = WalletState::from_ledger(&self.known_keys, ledger);
        self.wallet_states.insert(block, wallet_state);
    }

    pub fn balance(&self, tip: HeaderId, pk: PublicKey) -> Result<Option<Value>> {
        Ok(self.wallet_state_at(tip)?.balance(pk))
    }

    pub fn utxos_for_amount(
        &self,
        tip: HeaderId,
        amount: Value,
        pks: impl IntoIterator<Item = PublicKey>,
    ) -> Result<Option<Vec<Utxo>>> {
        Ok(self.wallet_state_at(tip)?.utxos_for_amount(amount, pks))
    }

    fn wallet_state_at(&self, tip: HeaderId) -> Result<&WalletState> {
        self.wallet_states
            .get(&tip)
            .ok_or(WalletError::UnknownBlock)
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

        let wallet = Wallet::from_lib([], genesis, ledger.clone());
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::from_lib([alice], genesis, ledger.clone());
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::from_lib([bob], genesis, ledger.clone());
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));

        let wallet = Wallet::from_lib([alice, bob], genesis, ledger);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));
    }

    #[test]
    fn test_sync() {
        let alice = pk(1);
        let bob = pk(2);

        let genesis = HeaderId::from([0; 32]);
        let block_1 = HeaderId::from([1; 32]);
        let block_2 = HeaderId::from([2; 32]);

        let genesis_ledger = LedgerState::from_utxos([]);

        let mut wallet = Wallet::from_lib([alice, bob], genesis, genesis_ledger);

        let ledger_1 = LedgerState::from_utxos([
            Utxo::new(tx_hash(0), 0, Note::new(100, alice)),
            Utxo::new(tx_hash(0), 1, Note::new(4, alice)),
        ]);

        wallet.apply_ledger(block_1, ledger_1);

        let ledger_2 = LedgerState::from_utxos([
            Utxo::new(tx_hash(0), 0, Note::new(20, bob)),
            Utxo::new(tx_hash(0), 1, Note::new(4, alice)),
        ]);

        wallet.apply_ledger(block_2, ledger_2);

        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        assert_eq!(wallet.balance(block_1, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(block_1, bob).unwrap(), None);

        assert_eq!(wallet.balance(block_2, alice).unwrap(), Some(4));
        assert_eq!(wallet.balance(block_2, bob).unwrap(), Some(20));
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

        let wallet = Wallet::from_lib([alice_1, alice_2, bob], genesis, ledger);

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
        // returns two UTXO's 3 & 4 NMO despite there existing a note of exactly 5 NMO available for alice
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

    // #[test]
    // fn test_utxos_for_amount_respects_spent_unconfirmed() {
    //     let alice = PublicKey::from([0u8; 32]);

    //     let ledger = LedgerState::from_utxos([
    //         Utxo::new(tx_hash(0), 0, Note::new(4, alice)),
    //         Utxo::new(tx_hash(0), 1, Note::new(3, alice)),
    //     ]);

    //     let genesis = HeaderId::from([0u8; 32]);

    //     let mut wallet = Wallet::new(genesis, ledger, [alice]);

    //     assert_eq!(
    //         wallet.utxos_for_amount(genesis, 1, [alice]).unwrap(),
    //         Some(vec![Utxo::new(tx_hash(0), 1, Note::new(3, alice))])
    //     );

    //     wallet.mark_spent_unconfirmed(Utxo::new(tx_hash(0), 1, Note::new(3, alice)).id());
    //     assert!(false);
    // }

    // #[test]
    // fn test_spent_unconfirmed_cleared_after_expiry() {
    //     let alice = PublicKey::from([0u8; 32]);

    //     let ledger = LedgerState::from_utxos([
    //         Utxo::new(tx_hash(0), 0, Note::new(4, alice)),
    //         Utxo::new(tx_hash(0), 1, Note::new(3, alice)),
    //     ]);

    //     let genesis = HeaderId::from([0u8; 32]);

    //     let mut wallet = Wallet::new(genesis, ledger, [alice]);

    //     panic!();
    // }

    // #[test]
    // fn test_balance_takes_into_account_unspent_unconfirmed_notes() {
    //     panic!();
    // }
}
