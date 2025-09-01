use std::{str::FromStr as _, sync::LazyLock};

use bytes::Bytes;
use groth16::{serde::serde_fr, Fr};
use num_bigint::BigUint;
use poseidon2::Digest;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::ZkHasher,
    mantle::{gas::GasConstants, keys::PublicKey, tx::TxHash, Transaction, TransactionHasher},
};

pub type Value = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoteId(#[serde(with = "serde_fr")] pub Fr);

impl NoteId {
    #[must_use]
    pub const fn as_fr(&self) -> &Fr {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> Bytes {
        self.0 .0 .0.iter().flat_map(|b| b.to_le_bytes()).collect()
    }
}

impl AsRef<Fr> for NoteId {
    fn as_ref(&self) -> &Fr {
        &self.0
    }
}

impl From<Fr> for NoteId {
    fn from(n: Fr) -> Self {
        Self(n)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct Note {
    pub value: Value,
    pub pk: PublicKey,
}

impl Note {
    #[must_use]
    pub const fn new(value: Value, pk: PublicKey) -> Self {
        Self { value, pk }
    }

    #[must_use]
    pub fn as_fr_components(&self) -> [Fr; 2] {
        [BigUint::from(self.value).into(), *self.pk.as_fr()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tx {
    pub inputs: Vec<NoteId>,
    pub outputs: Vec<Note>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Utxo {
    pub tx_hash: TxHash,
    pub output_index: usize,
    pub note: Note,
}

static NOMOS_NOTE_ID_V1: LazyLock<Fr> =
    // Constant for Fr(b"NOMOS_NOTE_ID_V1")
    LazyLock::new(|| {
        BigUint::from_str("65580641562429851895355409762135920462")
            .expect("BigUint should load from constant string")
            .into()
    });

impl Utxo {
    #[must_use]
    pub fn id(&self) -> NoteId {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9

        let mut hasher = ZkHasher::default();
        let tx_hash: Fr = *self.tx_hash.as_ref();
        let output_index: Fr =
            BigUint::from_bytes_le(self.output_index.to_le_bytes().as_slice()).into();
        let note_value: Fr =
            BigUint::from_bytes_le(self.note.value.to_le_bytes().as_slice()).into();
        let note_pk: Fr = self.note.pk.into();
        <ZkHasher as Digest>::update(&mut hasher, &NOMOS_NOTE_ID_V1);
        <ZkHasher as Digest>::update(&mut hasher, &tx_hash);
        <ZkHasher as Digest>::update(&mut hasher, &output_index);
        <ZkHasher as Digest>::update(&mut hasher, &note_value);
        <ZkHasher as Digest>::update(&mut hasher, &note_pk);

        let hash = hasher.finalize();
        NoteId(hash)
    }
}

static NOMOS_LEDGER_TXHASH_V1_FR: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_le(b"NOMOS_LEDGER_TXHASH_V1").into());

static INOUT_SEP_FR: LazyLock<Fr> = LazyLock::new(|| BigUint::from_bytes_le(b"INOUT_SEP").into());

impl Tx {
    #[must_use]
    pub const fn new(inputs: Vec<NoteId>, outputs: Vec<Note>) -> Self {
        Self { inputs, outputs }
    }

    #[must_use]
    pub fn as_signing_frs(&self) -> Vec<Fr> {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9
        let mut output = vec![*NOMOS_LEDGER_TXHASH_V1_FR];
        output.extend(self.inputs.iter().map(NoteId::as_fr));
        output.push(*INOUT_SEP_FR);
        output.extend(self.outputs.iter().flat_map(Note::as_fr_components));
        output
    }

    #[must_use]
    pub fn utxo_by_index(&self, index: usize) -> Option<Utxo> {
        self.outputs.get(index).map(|note| Utxo {
            tx_hash: self.hash(),
            output_index: index,
            note: *note,
        })
    }

    #[must_use]
    pub const fn execution_gas<Constants: GasConstants>(&self) -> u64 {
        Constants::LEDGER_TX
    }

    pub fn utxos(&self) -> impl Iterator<Item = Utxo> + '_ {
        let tx_hash = self.hash();
        self.outputs
            .iter()
            .enumerate()
            .map(move |(index, note)| Utxo {
                tx_hash,
                output_index: index,
                note: *note,
            })
    }
}

impl Transaction for Tx {
    const HASHER: TransactionHasher<Self> =
        |tx| <ZkHasher as Digest>::digest(&tx.as_signing_frs()).into();
    type Hash = TxHash;

    fn as_signing_frs(&self) -> Vec<Fr> {
        Self::as_signing_frs(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_utxo_by_index() {
        let pk0 = PublicKey::from(Fr::from(BigUint::from(0u8)));
        let pk1 = PublicKey::from(Fr::from(BigUint::from(1u8)));
        let pk2 = PublicKey::from(Fr::from(BigUint::from(2u8)));
        let tx = Tx {
            inputs: vec![NoteId(BigUint::from(0u8).into())],
            outputs: vec![
                Note::new(100, pk0),
                Note::new(200, pk1),
                Note::new(300, pk2),
            ],
        };
        assert_eq!(
            tx.utxo_by_index(0),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 0,
                note: Note::new(100, pk0),
            })
        );
        assert_eq!(
            tx.utxo_by_index(1),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 1,
                note: Note::new(200, pk1),
            })
        );
        assert_eq!(
            tx.utxo_by_index(2),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 2,
                note: Note::new(300, pk2),
            })
        );

        assert!(tx.utxo_by_index(3).is_none());
    }
}
