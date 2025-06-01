use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
};

use serde::{Deserialize, Serialize};

use crate::ops::Op;

// PLACEHOLDERS
pub type LedgerTx = ();
pub type OpProof = ();
pub type LedgerTxProof = ();

pub const MANTLE_HASH_VERSION: &[u8] = b"NOMOS_MANTLE_TXHASH_V1";

#[derive(Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    // temporary holder
    pub ledger_tx: PhantomData<LedgerTx>,
    pub gas_price: u64,
}

impl Hash for MantleTx {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(MANTLE_HASH_VERSION);
        self.ops.hash(state);
        self.gas_price.hash(state);
        self.ledger_tx.hash(state);
    }
}

#[derive(Serialize, Deserialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    pub ops_profs: Vec<OpProof>,
    pub ledger_tx_proof: LedgerTxProof,
}
