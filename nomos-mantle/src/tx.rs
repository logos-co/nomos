use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::ops::Op;

// PLACEHOLDERS
pub type LedgerTx = ();
pub type OpProof = ();
pub type LedgerTxProof = ();

#[derive(Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    // temporary holder
    pub ledger_tx: PhantomData<LedgerTx>,
    pub gas_price: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    pub ops_profs: Vec<OpProof>,
    pub ledger_tx_proof: LedgerTxProof,
}
