use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nomos_core::{
    header::HeaderId,
    mantle::{keys::PublicKey, NoteId, Utxo, Value},
};
use nomos_ledger::LedgerState;

use crate::{Result, WalletError};

struct WalletState {
    pk_index: HashMap<PublicKey, BTreeSet<NoteId>>,
    utxos: BTreeMap<NoteId, Utxo>,
}

impl WalletState {
    pub fn from_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        let utxos = BTreeMap::from_iter(utxos.into_iter().map(|utxo| (utxo.id(), utxo)));

        let mut pk_index: HashMap<PublicKey, BTreeSet<NoteId>> = HashMap::new();

        for (id, utxo) in &utxos {
            pk_index
                .entry(utxo.note.pk)
                .or_insert_with(BTreeSet::new)
                .insert(*id);
        }

        Self { pk_index, utxos }
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
}

struct Wallet {
    lib: HeaderId,
    known_keys: HashSet<PublicKey>,
    wallet_states: BTreeMap<HeaderId, WalletState>,
}

impl Wallet {
    pub fn new(
        lib: HeaderId,
        ledger: &LedgerState,
        known_keys: impl IntoIterator<Item = PublicKey>,
    ) -> Self {
        let known_keys: HashSet<PublicKey> = known_keys.into_iter().collect();

        let wallet_state = WalletState::from_utxos(
            ledger
                .latest_commitments()
                .utxos()
                .iter()
                .map(|(_, (utxo, _))| *utxo)
                .filter(|utxo| known_keys.contains(&utxo.note.pk)),
        );

        Self {
            lib,
            known_keys: known_keys.into_iter().collect(),
            wallet_states: [(lib, wallet_state)].into(),
        }
    }

    pub fn balance(&self, tip: HeaderId, pk: PublicKey) -> Result<Option<Value>> {
        Ok(self
            .wallet_states
            .get(&tip)
            .ok_or(WalletError::UnknownBlock)?
            .balance(pk))
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
            Utxo {
                tx_hash: tx_hash(0),
                output_index: 0,
                note: Note {
                    value: 100,
                    pk: alice,
                },
            },
            Utxo {
                tx_hash: tx_hash(0),
                output_index: 0,
                note: Note { value: 20, pk: bob },
            },
            Utxo {
                tx_hash: tx_hash(0),
                output_index: 0,
                note: Note {
                    value: 4,
                    pk: alice,
                },
            },
        ]);

        let wallet = Wallet::new(genesis, &ledger, []);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::new(genesis, &ledger, [alice]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), None);

        let wallet = Wallet::new(genesis, &ledger, [bob]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), None);
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));

        let wallet = Wallet::new(genesis, &ledger, [alice, bob]);
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(104));
        assert_eq!(wallet.balance(genesis, bob).unwrap(), Some(20));
    }
}
