use std::sync::LazyLock;

use bytes::Bytes;
use groth16::{serde::serde_fr, Fr};
use num_bigint::BigUint;
use poseidon2::{Digest, ZkHash};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::ZkHasher,
    mantle::{
        gas::{Gas, GasConstants, GasCost},
        ledger::Tx as LedgerTx,
        ops::{Op, OpProof},
        AuthenticatedMantleTx, Transaction, TransactionHasher,
    },
    proofs::zksig::{DummyZkSignature as ZkSignature, ZkSignatureProof},
};

/// The hash of a transaction
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TxHash(#[serde(with = "serde_fr")] pub ZkHash);

impl From<ZkHash> for TxHash {
    fn from(fr: ZkHash) -> Self {
        Self(fr)
    }
}

impl From<BigUint> for TxHash {
    fn from(value: BigUint) -> Self {
        Self(value.into())
    }
}

impl From<TxHash> for ZkHash {
    fn from(hash: TxHash) -> Self {
        hash.0
    }
}

impl AsRef<ZkHash> for TxHash {
    fn as_ref(&self) -> &ZkHash {
        &self.0
    }
}

impl TxHash {
    /// For testing purposes
    #[cfg(test)]
    pub fn random(mut rng: impl rand::RngCore) -> Self {
        Self(BigUint::from(rng.next_u64()).into())
    }

    #[must_use]
    pub fn as_signing_bytes(&self) -> Bytes {
        self.0 .0 .0.iter().flat_map(|b| b.to_le_bytes()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    pub ledger_tx: LedgerTx,
    pub execution_gas_price: Gas,
    pub storage_gas_price: Gas,
}

static NOMOS_MANTLE_TXHASH_V1_FR: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_be(b"NOMOS_MANTLE_TXHASH_V1").into());

static END_OPS_FR: LazyLock<Fr> = LazyLock::new(|| BigUint::from_bytes_be(b"END_OPS").into());

impl Transaction for MantleTx {
    const HASHER: TransactionHasher<Self> =
        |tx| <ZkHasher as Digest>::digest(&tx.as_signing_frs()).into();
    type Hash = TxHash;

    fn as_signing_frs(&self) -> Vec<Fr> {
        // constant and structure as defined in the Mantle specification:
        // https://www.notion.so/Mantle-Specification-21c261aa09df810c8820fab1d78b53d9

        let mut output: Vec<Fr> = vec![*NOMOS_MANTLE_TXHASH_V1_FR];
        output.extend(self.ops.iter().flat_map(Op::as_signing_fr));
        output.push(*END_OPS_FR);
        output.push(BigUint::from(self.storage_gas_price).into());
        output.push(BigUint::from(self.execution_gas_price).into());
        output.extend(self.ledger_tx.as_signing_frs());
        output
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
    // TODO: make this more efficient
    pub ops_profs: Vec<Option<OpProof>>,
    pub ledger_tx_proof: ZkSignature,
}

impl Transaction for SignedMantleTx {
    const HASHER: TransactionHasher<Self> =
        |tx| <ZkHasher as Digest>::digest(&tx.as_signing_frs()).into();
    type Hash = TxHash;

    fn as_signing_frs(&self) -> Vec<Fr> {
        self.mantle_tx.as_signing_frs()
    }
}

impl AuthenticatedMantleTx for SignedMantleTx {
    fn mantle_tx(&self) -> &MantleTx {
        &self.mantle_tx
    }

    fn ledger_tx_proof(&self) -> &impl ZkSignatureProof {
        &self.ledger_tx_proof
    }

    fn ops_with_proof(&self) -> impl Iterator<Item = (&Op, Option<&OpProof>)> {
        self.mantle_tx
            .ops
            .iter()
            .zip(self.ops_profs.iter().map(Option::as_ref))
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
