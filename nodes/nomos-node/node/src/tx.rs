use std::hash::Hash;

use bytes::Bytes;
use nomos_core::tx::{Transaction, TransactionHasher};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct Tx(pub String);

fn hash_tx(tx: &Tx) -> String {
    tx.0.clone()
}

impl Transaction for Tx {
    const HASHER: TransactionHasher<Self> = hash_tx;
    type Hash = String;

    fn as_bytes(&self) -> Bytes {
        self.0.as_bytes().to_vec().into()
    }
}
