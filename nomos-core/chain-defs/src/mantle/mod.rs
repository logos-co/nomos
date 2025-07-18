use std::hash::Hash;

use bytes::Bytes;
use thiserror::Error;

pub mod gas;
pub mod keys;
pub mod ledger;
pub mod merkle;
#[cfg(feature = "mock")]
pub mod mock;
pub mod ops;
pub mod select;
pub mod tx;

pub use gas::{GasConstants, GasCost};
pub use ledger::{Note, NoteId, Utxo, Value};
pub use tx::{MantleTx, SignedMantleTx, TxHash};

use crate::proofs::zksig::ZkSignatureProof;

pub type TransactionHasher<T> = fn(&T) -> <T as Transaction>::Hash;

pub trait Transaction {
    const HASHER: TransactionHasher<Self>;
    type Hash: Hash + Eq + Clone;
    fn hash(&self) -> Self::Hash {
        Self::HASHER(self)
    }
    /// Returns the bytes that are used to form a signature of a transaction.
    ///
    /// The resulting bytes are then used by the `HASHER`
    /// to produce the transaction's unique hash, which is what is typically
    /// signed by the transaction originator.
    fn as_sign_bytes(&self) -> Bytes;
}

pub trait AuthenticatedMantleTx: Transaction<Hash = TxHash> + GasCost {
    /// Returns the underlying `MantleTx` that this transaction represents.
    fn mantle_tx(&self) -> &MantleTx;

    /// Returns the proof of the ledger transaction
    fn ledger_tx_proof(&self) -> &impl ZkSignatureProof;
}

impl<T: Transaction> Transaction for &T {
    const HASHER: TransactionHasher<Self> = |tx| T::HASHER(tx);
    type Hash = T::Hash;

    fn as_sign_bytes(&self) -> Bytes {
        T::as_sign_bytes(self)
    }
}

impl<T: AuthenticatedMantleTx> AuthenticatedMantleTx for &T {
    fn mantle_tx(&self) -> &MantleTx {
        T::mantle_tx(self)
    }

    fn ledger_tx_proof(&self) -> &impl ZkSignatureProof {
        T::ledger_tx_proof(self)
    }
}

pub trait TxSelect {
    type Tx: Transaction;
    type Settings: Clone;
    fn new(settings: Self::Settings) -> Self;

    fn select_tx_from<'i, I: Iterator<Item = Self::Tx> + 'i>(
        &self,
        txs: I,
    ) -> impl Iterator<Item = Self::Tx> + 'i;
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Risc0 failed to prove execution of the zkvm")]
    Risc0ProofFailed(#[from] anyhow::Error),
    #[error("Invalid witness")]
    InvalidWitness,
}
