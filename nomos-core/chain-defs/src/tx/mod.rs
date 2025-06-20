use std::hash::Hash;

use blake2::Digest as _;
use bytes::Bytes;
use thiserror::Error;

use crate::crypto::Blake2b;

pub mod bundle;
#[cfg(feature = "mock")]
pub mod mock;
pub mod select;

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

pub trait TxSelect {
    type Tx: Transaction;
    type Settings: Clone;
    fn new(settings: Self::Settings) -> Self;

    fn select_tx_from<'i, I: Iterator<Item = Self::Tx> + 'i>(
        &self,
        txs: I,
    ) -> impl Iterator<Item = Self::Tx> + 'i;
}

pub enum Tx {
    Bundle(bundle::Bundle),
}

impl Transaction for Tx {
    const HASHER: TransactionHasher<Self> = |tx| Blake2b::digest(tx.as_sign_bytes()).into();
    type Hash = [u8; 32];

    fn as_sign_bytes(&self) -> Bytes {
        match self {
            Self::Bundle(bundle) => bundle.as_bytes(),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Risc0 failed to prove execution of the zkvm")]
    Risc0ProofFailed(#[from] anyhow::Error),
    #[error("Invalid witness")]
    InvalidWitness,
}
