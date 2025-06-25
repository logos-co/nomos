use blake2::Digest as _;
use serde::{Deserialize, Serialize};

use crate::{
    mantle::{
        gas::{Gas, GasConstants, GasPrice},
        ledger::Tx as LedgerTx,
        ops::Op,
        Transaction, TransactionHasher,
    },
    utils::serde_bytes_newtype,
};

pub type OpProof = ();
pub type ZkSignature = ();
/// The hash of a transaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub struct TxHash(pub [u8; 32]);

serde_bytes_newtype!(TxHash, 32);

impl From<[u8; 32]> for TxHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl TxHash {
    /// For testing purposes
    #[cfg(test)]
    pub fn random(mut rng: impl rand::RngCore) -> Self {
        let mut sk = [0u8; 32];
        rng.fill_bytes(&mut sk);
        Self(sk)
    }

    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl GasPrice for LedgerTx {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        // TODO: properly implement this when adding the ledger tx,
        // for now making every tx too expensive so it would blow up its usage.
        u64::MAX
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    pub ledger_tx: LedgerTx,
    pub gas_price: Gas,
}

impl Transaction for MantleTx {
    const HASHER: TransactionHasher<Self> =
        |tx| <[u8; 32]>::from(crate::crypto::Hasher::digest(tx.as_sign_bytes())).into();
    type Hash = TxHash;

    fn as_sign_bytes(&self) -> bytes::Bytes {
        // constant and structure as defined in the Mantle specification:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9
        const NOMOS_MANTLE_TXHASH_V1: &[u8] = b"NOMOS_MANTLE_TXHASH_V1";
        const END_OPS: &[u8] = b"END_OPS";

        let mut buff = bytes::BytesMut::new();
        buff.extend_from_slice(NOMOS_MANTLE_TXHASH_V1);
        for op in &self.ops {
            buff.extend_from_slice(op.as_sign_bytes().as_ref());
        }
        buff.extend_from_slice(END_OPS);
        buff.extend_from_slice(self.gas_price.to_le_bytes().as_ref());
        buff.extend_from_slice(self.ledger_tx.as_sign_bytes().as_ref());
        buff.freeze()
    }
}

impl From<SignedMantleTx> for MantleTx {
    fn from(signed_tx: SignedMantleTx) -> Self {
        signed_tx.mantle_tx
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    pub ops_profs: Vec<OpProof>,
    pub ledger_tx_proof: ZkSignature,
}

impl Transaction for SignedMantleTx {
    const HASHER: TransactionHasher<Self> =
        |tx| <[u8; 32]>::from(crate::crypto::Hasher::digest(tx.as_sign_bytes())).into();
    type Hash = TxHash;

    fn as_sign_bytes(&self) -> bytes::Bytes {
        self.mantle_tx.as_sign_bytes()
    }
}

impl GasPrice for MantleTx {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        let ops_gas: Gas = self.ops.iter().map(GasPrice::gas_price::<Constants>).sum();
        let ledger_tx_gas = self.ledger_tx.gas_price::<Constants>();
        ops_gas + ledger_tx_gas
    }
}

impl GasPrice for SignedMantleTx {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        self.mantle_tx.gas_price::<Constants>()
    }
}
