use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::Digest as _,
    mantle::{gas::GasConstants, keys::PublicKey, tx::TxHash, Transaction, TransactionHasher},
    utils::serde_bytes_newtype,
};

pub type Value = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(pub [u8; 32]);

serde_bytes_newtype!(NoteId, 32);

impl NoteId {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl AsRef<[u8; 32]> for NoteId {
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl AsRef<[u8]> for NoteId {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
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
    pub fn to_bytes(&self) -> [u8; 40] {
        let mut bytes = [0u8; 40];
        bytes[..8].copy_from_slice(&self.value.to_le_bytes());
        bytes[8..].copy_from_slice(&self.pk.as_bytes()[..]);
        bytes
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

impl Utxo {
    #[must_use]
    pub fn id(&self) -> NoteId {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9
        const NOMOS_NOTE_ID_V1: &[u8] = b"NOMOS_NOTE_ID_V1";

        let mut hasher = crate::crypto::Hasher::new();

        hasher.update(NOMOS_NOTE_ID_V1);
        hasher.update(self.tx_hash.0);
        hasher.update(self.output_index.to_le_bytes());
        hasher.update(self.note.value.to_le_bytes());
        hasher.update(self.note.pk.as_bytes());

        let hash = hasher.finalize();
        NoteId(hash.into())
    }
}

impl Tx {
    #[must_use]
    pub const fn new(inputs: Vec<NoteId>, outputs: Vec<Note>) -> Self {
        Self { inputs, outputs }
    }

    #[must_use]
    pub fn as_sign_bytes(&self) -> Bytes {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9
        const NOMOS_LEDGER_TXHASH_V1: &[u8] = b"NOMOS_LEDGER_TXHASH_V1";
        const INOUT_SEP: &[u8] = b"INOUT_SEP";

        let mut bytes = BytesMut::from(NOMOS_LEDGER_TXHASH_V1);

        for input in &self.inputs {
            bytes.extend(input.as_bytes());
        }

        bytes.extend_from_slice(INOUT_SEP);

        for output in &self.outputs {
            bytes.extend_from_slice(output.to_bytes().as_ref());
        }

        bytes.freeze()
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
        |tx| <[u8; 32]>::from(crate::crypto::Hasher::digest(tx.as_sign_bytes())).into();
    type Hash = TxHash;

    fn as_sign_bytes(&self) -> Bytes {
        self.as_sign_bytes()
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_utxo_by_index() {
        let tx = Tx {
            inputs: vec![NoteId([0; 32])],
            outputs: vec![
                Note::new(100, [0; 32].into()),
                Note::new(200, [1; 32].into()),
                Note::new(300, [2; 32].into()),
            ],
        };
        assert_eq!(
            tx.utxo_by_index(0),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 0,
                note: Note::new(100, [0; 32].into()),
            })
        );
        assert_eq!(
            tx.utxo_by_index(1),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 1,
                note: Note::new(200, [1; 32].into()),
            })
        );
        assert_eq!(
            tx.utxo_by_index(2),
            Some(Utxo {
                tx_hash: tx.hash(),
                output_index: 2,
                note: Note::new(300, [2; 32].into()),
            })
        );

        assert!(tx.utxo_by_index(3).is_none());
    }
}
