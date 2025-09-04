use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nomos_core::{
    header::HeaderId,
    mantle::{keys::PublicKey, NoteId, Utxo, Value},
};
use nomos_ledger::LedgerState;

use crate::{Result, WalletError};

struct WalletState {
    pk_index: HashMap<PublicKey, BTreeSet<NoteId>>,
    ledger: LedgerState,
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
            pk_index
                .entry(utxo.note.pk)
                .or_insert_with(BTreeSet::new)
                .insert(*id);
        }

        Self { pk_index, ledger }
    }

    pub fn utxos(&self) -> Vec<Utxo> {
        let utxotree = self.ledger.latest_commitments().utxos();
        self.pk_index
            .iter()
            .flat_map(|(_pk, ids)| {
                ids.iter()
                    .filter_map(|id| utxotree.get(id))
                    .map(|(utxo, _pos)| *utxo)
            })
            .collect()
    }

    pub fn balance(&self, pk: PublicKey) -> Option<Value> {
        let balance = self
            .pk_index
            .get(&pk)?
            .iter()
            .map(|id| {
                self.ledger
                    .latest_commitments()
                    .utxos()
                    .get(id)
                    .expect("missing utxo entry")
                    .0
                    .note
                    .value
            })
            .sum();

        Some(balance)
    }
}

struct Wallet {
    known_keys: HashSet<PublicKey>,
    wallet_states: BTreeMap<HeaderId, WalletState>,
}

impl Wallet {
    pub fn new(
        block: HeaderId,
        ledger: LedgerState,
        known_keys: impl IntoIterator<Item = PublicKey>,
    ) -> Self {
        let known_keys: HashSet<PublicKey> = known_keys.into_iter().collect();

        let wallet_state = WalletState::from_ledger(&known_keys, ledger);

        Self {
            known_keys: known_keys.into_iter().collect(),
            wallet_states: [(block, wallet_state)].into(),
        }
    }

    pub fn apply_ledger(&mut self, block: HeaderId, ledger: LedgerState) {
        let wallet_state = WalletState::from_ledger(&self.known_keys, ledger);
        self.wallet_states.insert(block, wallet_state);
    }

    pub fn balance(&self, tip: HeaderId, pk: PublicKey) -> Result<Option<Value>> {
        let balance = self.wallet_state_at(tip)?.balance(pk);

        Ok(balance)
    }

    pub fn utxos_for_amount(
        &self,
        tip: HeaderId,
        amount: Value,
        pks: impl IntoIterator<Item = PublicKey>,
    ) -> Result<Option<Vec<Utxo>>> {
        let wallet_state = self.wallet_state_at(tip)?;

        let eligible_pks: HashSet<PublicKey> = pks.into_iter().collect();
        let mut utxos: Vec<Utxo> = wallet_state
            .utxos()
            .into_iter()
            .filter(|utxo| eligible_pks.contains(&utxo.note.pk))
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
            Ok(None)
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

            Ok(Some(selected_utxos))
        }
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

        let wallet = Wallet::new(genesis, ledger.clone(), []);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::new(genesis, ledger.clone(), [alice]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::new(genesis, ledger.clone(), [bob]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));

        let wallet = Wallet::new(genesis, ledger, [alice, bob]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));
    }

    #[test]
    fn test_ledger_sync() {
        let alice = pk(1);
        let bob = pk(2);

        let genesis = HeaderId::from([0; 32]);
        let block_1 = HeaderId::from([1; 32]);
        let block_2 = HeaderId::from([2; 32]);

        let genesis_ledger = LedgerState::from_utxos([]);

        let mut wallet = Wallet::new(genesis, genesis_ledger, [alice, bob]);

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

        let wallet = Wallet::new(genesis, ledger, [alice_1, alice_2, bob]);

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
