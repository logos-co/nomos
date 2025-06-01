use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
};

use serde::{Deserialize, Serialize};

use crate::{
    gas::{Gas, GasConstants, GasPrice},
    ops::Op,
};

// PLACEHOLDERS
pub type LedgerTx = PhantomData<()>;

impl GasPrice for LedgerTx {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        // TODO: properly implement this when adding the ledger tx,
        // for now making every tx too expensive so it would blow up its usage.
        u64::MAX
    }
}
pub type OpProof = ();
pub type LedgerTxProof = ();

pub const MANTLE_HASH_VERSION: &[u8] = b"NOMOS_MANTLE_TXHASH_V1";

#[derive(Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    // temporary holder
    pub ledger_tx: LedgerTx,
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

impl GasPrice for MantleTx {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        let ops_gas: Gas = self.ops.iter().map(GasPrice::gas_price::<Constants>).sum();
        let ledger_tx_gas = self.ledger_tx.gas_price::<Constants>();
        ops_gas + ledger_tx_gas
    }
}

#[derive(Serialize, Deserialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    pub ops_profs: Vec<OpProof>,
    pub ledger_tx_proof: LedgerTxProof,
}
