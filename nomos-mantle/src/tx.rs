use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
};

use blake2::Digest as _;
use nomos_core::{tx::TransactionHasher, wire};
use serde::{Deserialize, Serialize};

use crate::{
    gas::{Gas, GasConstants, GasPrice},
    ops::Op,
};

pub type TxHash = [u8; 32];

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
pub type ZkSignature = ();

pub const MANTLE_HASH_VERSION: &[u8] = b"NOMOS_MANTLE_TXHASH_V1";

#[derive(Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    // temporary holder
    pub ledger_tx: LedgerTx,
    pub gas_price: Gas,
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

impl nomos_core::tx::Transaction for MantleTx {
    const HASHER: TransactionHasher<Self> = |tx| blake2::Blake2b::digest(tx.as_bytes()).into();
    type Hash = TxHash;

    fn as_bytes(&self) -> bytes::Bytes {
        let mut buff = bytes::BytesMut::new();
        buff.extend_from_slice(MANTLE_HASH_VERSION);
        buff.extend_from_slice(wire::serialize(&self.ops).unwrap().as_ref());
        buff.extend_from_slice(wire::serialize(&self.gas_price).unwrap().as_ref());
        buff.extend_from_slice(wire::serialize(&self.ledger_tx).unwrap().as_ref());
        buff.freeze()
    }
}

#[derive(Serialize, Deserialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    pub ops_profs: Vec<OpProof>,
    pub ledger_tx_proof: ZkSignature,
}
