use blake2::Digest as _;
use serde::{Deserialize, Serialize};

use crate::{
    mantle::{
        gas::{Gas, GasConstants, GasCost},
        ledger::Tx as LedgerTx,
        ops::Op,
        AuthenticatedMantleTx, Transaction, TransactionHasher,
    },
    proofs::zksig::{DummyZkSignature as ZkSignature, ZkSignatureProof},
    utils::serde_bytes_newtype,
};

pub type OpProof = ();
/// The hash of a transaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub struct TxHash(pub [u8; 32]);

serde_bytes_newtype!(TxHash, 32);

impl From<[u8; 32]> for TxHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<TxHash> for [u8; 32] {
    fn from(hash: TxHash) -> Self {
        hash.0
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    pub ledger_tx: LedgerTx,
    pub execution_gas_price: Gas,
    pub storage_gas_price: Gas,
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

        buff.extend_from_slice(self.storage_gas_price.to_le_bytes().as_ref());
        buff.extend_from_slice(self.execution_gas_price.to_le_bytes().as_ref());

        buff.extend_from_slice(&self.ledger_tx.hash().0);
        buff.freeze()
    }
}

impl From<SignedMantleTx> for MantleTx {
    fn from(signed_tx: SignedMantleTx) -> Self {
        signed_tx.mantle_tx
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl AuthenticatedMantleTx for SignedMantleTx {
    fn mantle_tx(&self) -> &MantleTx {
        &self.mantle_tx
    }

    fn ledger_tx_proof(&self) -> &impl ZkSignatureProof {
        &self.ledger_tx_proof
    }
}

impl SignedMantleTx {
    fn serialized_size(&self) -> u64 {
        use bincode::Options as _;
        // TODO: we need a more universal size estimation, but that means complete
        // control over serialization which requires a rework of the wire module
        crate::wire::bincode::OPTIONS
            .serialized_size(&self)
            .expect("Failed to serialize signed mantle tx")
    }
}

impl GasCost for SignedMantleTx {
    fn gas_cost<Constants: GasConstants>(&self) -> Gas {
        let execution_gas = self
            .mantle_tx
            .ops
            .iter()
            .map(Op::execution_gas::<Constants>)
            .sum::<Gas>()
            + self.mantle_tx.ledger_tx.execution_gas::<Constants>();
        let storage_gas = self.serialized_size();
        let da_gas_cost = self.mantle_tx.ops.iter().map(Op::da_gas_cost).sum::<Gas>();

        execution_gas * self.mantle_tx.execution_gas_price
            + storage_gas * self.mantle_tx.storage_gas_price
            + da_gas_cost
    }
}
