use std::sync::LazyLock;

use bytes::Bytes;
use groth16::{Fr, fr_from_bytes, serde::serde_fr};
use num_bigint::BigUint;
use poseidon2::{Digest, ZkHash};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::ZkHasher,
    mantle::{
        AuthenticatedMantleTx, Transaction, TransactionHasher,
        gas::{Gas, GasConstants, GasCost},
        ledger::Tx as LedgerTx,
        ops::{Op, OpProof},
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
        self.0.0.0.iter().flat_map(|b| b.to_le_bytes()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    pub ledger_tx: LedgerTx,
    pub execution_gas_price: Gas,
    pub storage_gas_price: Gas,
}

static NOMOS_MANTLE_TXHASH_V1_FR: LazyLock<Fr> = LazyLock::new(|| {
    fr_from_bytes(b"NOMOS_MANTLE_TXHASH_V1").expect("Constant should be valid Fr")
});

static END_OPS_FR: LazyLock<Fr> =
    LazyLock::new(|| fr_from_bytes(b"END_OPS").expect("Constant should be valid Fr"));

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignedMantleTx {
    pub mantle_tx: MantleTx,
    // TODO: make this more efficient
    pub ops_proofs: Vec<Option<OpProof>>,
    pub ledger_tx_proof: ZkSignature,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VerificationError {
    #[error("Invalid signature for operation at index {op_index}")]
    InvalidSignature { op_index: usize },
    #[error("Missing required proof for {op_type} operation at index {op_index}")]
    MissingProof {
        op_type: &'static str,
        op_index: usize,
    },
    #[error("Incorrect proof type for {op_type} operation at index {op_index}")]
    IncorrectProofType {
        op_type: &'static str,
        op_index: usize,
    },
}

impl SignedMantleTx {
    /// Create a new `SignedMantleTx` and verify that all required proofs are
    /// present and valid.
    ///
    /// This enforces at construction time that:
    /// - `ChannelBlob` operations have a valid Ed25519 signature from the
    ///   declared signer
    /// - `ChannelInscribe` operations have a valid Ed25519 signature from the
    ///   declared signer
    pub fn new(
        mantle_tx: MantleTx,
        ops_proofs: Vec<Option<OpProof>>,
        ledger_tx_proof: ZkSignature,
    ) -> Result<Self, VerificationError> {
        let tx = Self {
            mantle_tx,
            ops_proofs,
            ledger_tx_proof,
        };
        tx.verify_ops_proofs()?;
        Ok(tx)
    }

    /// Create a `SignedMantleTx` without verifying proofs.
    /// This should only be used for `GenesisTx` or in tests.
    #[doc(hidden)]
    #[cfg(any(test, debug_assertions))]
    #[must_use]
    pub const fn new_unverified(
        mantle_tx: MantleTx,
        ops_proofs: Vec<Option<OpProof>>,
        ledger_tx_proof: ZkSignature,
    ) -> Self {
        Self {
            mantle_tx,
            ops_proofs,
            ledger_tx_proof,
        }
    }

    fn verify_ops_proofs(&self) -> Result<(), VerificationError> {
        use ed25519::signature::Verifier as _;

        let tx_hash = self.hash();
        let tx_hash_bytes = tx_hash.as_signing_bytes();

        for (idx, (op, proof)) in self
            .mantle_tx
            .ops
            .iter()
            .zip(self.ops_proofs.iter())
            .enumerate()
        {
            match op {
                Op::ChannelBlob(blob_op) => {
                    // Blob operations require an Ed25519 signature
                    let Some(OpProof::Ed25519Sig(sig)) = proof else {
                        if proof.is_none() {
                            return Err(VerificationError::MissingProof {
                                op_type: "ChannelBlob",
                                op_index: idx,
                            });
                        }
                        return Err(VerificationError::IncorrectProofType {
                            op_type: "ChannelBlob",
                            op_index: idx,
                        });
                    };

                    blob_op
                        .signer
                        .verify(tx_hash_bytes.as_ref(), sig)
                        .map_err(|_| VerificationError::InvalidSignature { op_index: idx })?;
                }
                Op::ChannelInscribe(inscribe_op) => {
                    // Inscription operations require an Ed25519 signature
                    let Some(OpProof::Ed25519Sig(sig)) = proof else {
                        if proof.is_none() {
                            return Err(VerificationError::MissingProof {
                                op_type: "ChannelInscribe",
                                op_index: idx,
                            });
                        }
                        return Err(VerificationError::IncorrectProofType {
                            op_type: "ChannelInscribe",
                            op_index: idx,
                        });
                    };

                    inscribe_op
                        .signer
                        .verify(tx_hash_bytes.as_ref(), sig)
                        .map_err(|_| VerificationError::InvalidSignature { op_index: idx })?;
                }
                // Other operations are checked by the ledger or don't require verification here
                _ => {}
            }
        }

        Ok(())
    }

    fn serialized_size(&self) -> u64 {
        <Self as crate::codec::SerdeOp>::serialized_size(self)
            .expect("Failed to calculate serialized size for signed mantle tx")
    }
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
            .zip(self.ops_proofs.iter().map(Option::as_ref))
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

impl<'de> Deserialize<'de> for SignedMantleTx {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SignedMantleTxHelper {
            mantle_tx: MantleTx,
            ops_proofs: Vec<Option<OpProof>>,
            ledger_tx_proof: ZkSignature,
        }

        let helper = SignedMantleTxHelper::deserialize(deserializer)?;
        Self::new(helper.mantle_tx, helper.ops_proofs, helper.ledger_tx_proof)
            .map_err(serde::de::Error::custom)
    }
}
