use std::sync::LazyLock;

use bytes::Bytes;
use groth16::{Fr, fr_from_bytes, fr_to_bytes, serde::serde_fr};
use num_bigint::BigUint;
use poseidon2::{Digest, ZkHash};
use serde::{Deserialize, Serialize};

use crate::{
    codec::SerializeOp as _,
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

impl From<TxHash> for Bytes {
    fn from(tx_hash: TxHash) -> Self {
        Self::copy_from_slice(&fr_to_bytes(&tx_hash.0))
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
    #[error("Number of proofs ({proofs_count}) does not match number of operations ({ops_count})")]
    ProofCountMismatch {
        ops_count: usize,
        proofs_count: usize,
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

    // TODO: might drop proofs after verification
    fn verify_ops_proofs(&self) -> Result<(), VerificationError> {
        use ed25519::signature::Verifier as _;

        // Check that we have the same number of proofs as ops
        if self.mantle_tx.ops.len() != self.ops_proofs.len() {
            return Err(VerificationError::ProofCountMismatch {
                ops_count: self.mantle_tx.ops.len(),
                proofs_count: self.ops_proofs.len(),
            });
        }

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
        self.bytes_size()
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
            .map(|(op, proof)| {
                if matches!(op, Op::ChannelBlob(_) | Op::ChannelInscribe(_)) {
                    (op, None)
                } else {
                    (op, proof)
                }
            })
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

#[cfg(test)]
mod tests {
    use ed25519::signature::Signer as _;
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::{
        mantle::{
            ledger::Tx as LedgerTx,
            ops::channel::{blob::BlobOp, inscribe::InscriptionOp},
        },
        proofs::zksig::ZkSignaturePublic,
    };

    fn dummy_zk_signature() -> ZkSignature {
        ZkSignature::prove(&ZkSignaturePublic {
            msg_hash: Fr::default(),
            pks: vec![],
        })
    }

    fn create_test_mantle_tx(ops: Vec<Op>) -> MantleTx {
        MantleTx {
            ops,
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 1,
            storage_gas_price: 1,
        }
    }

    fn create_test_blob_op(signing_key: &SigningKey) -> BlobOp {
        BlobOp {
            channel: [0; 32].into(),
            blob: [0; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            parent: [0; 32].into(),
            signer: signing_key.verifying_key(),
        }
    }

    fn create_test_inscribe_op(signing_key: &SigningKey) -> InscriptionOp {
        InscriptionOp {
            channel_id: [0; 32].into(),
            inscription: vec![1, 2, 3],
            parent: [0; 32].into(),
            signer: signing_key.verifying_key(),
        }
    }

    #[test]
    fn test_signed_mantle_tx_new_with_valid_blob_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        // Sign the transaction hash
        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = signing_key.sign(&tx_hash);

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            dummy_zk_signature(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_signed_mantle_tx_new_with_valid_inscribe_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let inscribe_op = create_test_inscribe_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelInscribe(inscribe_op)]);

        // Sign the transaction hash
        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = signing_key.sign(&tx_hash);

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            dummy_zk_signature(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_signed_mantle_tx_new_missing_blob_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        let result = SignedMantleTx::new(mantle_tx, vec![None], dummy_zk_signature());

        assert!(matches!(
            result,
            Err(VerificationError::MissingProof {
                op_type: "ChannelBlob",
                op_index: 0
            })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_missing_inscribe_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let inscribe_op = create_test_inscribe_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelInscribe(inscribe_op)]);

        let result = SignedMantleTx::new(mantle_tx, vec![None], dummy_zk_signature());

        assert!(matches!(
            result,
            Err(VerificationError::MissingProof {
                op_type: "ChannelInscribe",
                op_index: 0
            })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_invalid_blob_signature() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let wrong_signing_key = SigningKey::from_bytes(&[2; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        // Sign with wrong key
        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = wrong_signing_key.sign(&tx_hash);

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            dummy_zk_signature(),
        );

        assert!(matches!(
            result,
            Err(VerificationError::InvalidSignature { op_index: 0 })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_invalid_inscribe_signature() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let wrong_signing_key = SigningKey::from_bytes(&[2; 32]);
        let inscribe_op = create_test_inscribe_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelInscribe(inscribe_op)]);

        // Sign with wrong key
        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = wrong_signing_key.sign(&tx_hash);

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            dummy_zk_signature(),
        );

        assert!(matches!(
            result,
            Err(VerificationError::InvalidSignature { op_index: 0 })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_incorrect_blob_proof_type() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        // Use wrong proof type
        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::ZkSig(dummy_zk_signature()))],
            dummy_zk_signature(),
        );

        assert!(matches!(
            result,
            Err(VerificationError::IncorrectProofType {
                op_type: "ChannelBlob",
                op_index: 0
            })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_incorrect_inscribe_proof_type() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let inscribe_op = create_test_inscribe_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelInscribe(inscribe_op)]);

        // Use wrong proof type
        let result = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::ZkSig(dummy_zk_signature()))],
            dummy_zk_signature(),
        );

        assert!(matches!(
            result,
            Err(VerificationError::IncorrectProofType {
                op_type: "ChannelInscribe",
                op_index: 0
            })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_new_multiple_ops_valid() {
        let signing_key1 = SigningKey::from_bytes(&[1; 32]);
        let signing_key2 = SigningKey::from_bytes(&[2; 32]);

        let blob_op = create_test_blob_op(&signing_key1);
        let inscribe_op = create_test_inscribe_op(&signing_key2);

        let mantle_tx = create_test_mantle_tx(vec![
            Op::ChannelBlob(blob_op),
            Op::ChannelInscribe(inscribe_op),
        ]);

        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let sig1 = signing_key1.sign(&tx_hash);
        let sig2 = signing_key2.sign(&tx_hash);

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![
                Some(OpProof::Ed25519Sig(sig1)),
                Some(OpProof::Ed25519Sig(sig2)),
            ],
            dummy_zk_signature(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_signed_mantle_tx_new_multiple_ops_one_invalid() {
        let signing_key1 = SigningKey::from_bytes(&[1; 32]);
        let signing_key2 = SigningKey::from_bytes(&[2; 32]);
        let wrong_key = SigningKey::from_bytes(&[3; 32]);

        let blob_op = create_test_blob_op(&signing_key1);
        let inscribe_op = create_test_inscribe_op(&signing_key2);

        let mantle_tx = create_test_mantle_tx(vec![
            Op::ChannelBlob(blob_op),
            Op::ChannelInscribe(inscribe_op),
        ]);

        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let sig1 = signing_key1.sign(&tx_hash);
        let sig2 = wrong_key.sign(&tx_hash); // Wrong signature

        let result = SignedMantleTx::new(
            mantle_tx,
            vec![
                Some(OpProof::Ed25519Sig(sig1)),
                Some(OpProof::Ed25519Sig(sig2)),
            ],
            dummy_zk_signature(),
        );

        assert!(matches!(
            result,
            Err(VerificationError::InvalidSignature { op_index: 1 })
        ));
    }

    #[test]
    fn test_signed_mantle_tx_deserialize_with_valid_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = signing_key.sign(&tx_hash);

        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            dummy_zk_signature(),
        )
        .unwrap();

        // Serialize and deserialize
        let serialized = serde_json::to_string(&signed_tx).unwrap();
        let deserialized: Result<SignedMantleTx, _> = serde_json::from_str(&serialized);

        assert!(deserialized.is_ok());
        assert_eq!(deserialized.unwrap(), signed_tx);
    }

    #[test]
    fn test_signed_mantle_tx_deserialize_with_missing_proof() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        let helper = SignedMantleTx {
            mantle_tx,
            ops_proofs: vec![None],
            ledger_tx_proof: dummy_zk_signature(),
        };

        let serialized = serde_json::to_string(&helper).unwrap();
        let deserialized: Result<SignedMantleTx, _> = serde_json::from_str(&serialized);

        assert!(deserialized.is_err());
        let err_msg = deserialized.unwrap_err().to_string();
        assert!(err_msg.contains("MissingProof") || err_msg.contains("ChannelBlob"));
    }

    #[test]
    fn test_signed_mantle_tx_deserialize_with_invalid_signature() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let wrong_key = SigningKey::from_bytes(&[2; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);

        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let wrong_signature = wrong_key.sign(&tx_hash);

        let helper = SignedMantleTx {
            mantle_tx,
            ops_proofs: vec![Some(OpProof::Ed25519Sig(wrong_signature))],
            ledger_tx_proof: dummy_zk_signature(),
        };

        let serialized = serde_json::to_string(&helper).unwrap();
        let deserialized: Result<SignedMantleTx, _> = serde_json::from_str(&serialized);

        assert!(deserialized.is_err());
        let err_msg = deserialized.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid signature"));
    }

    #[test]
    fn test_signed_mantle_tx_new_proof_count_mismatch() {
        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = create_test_blob_op(&signing_key);
        let mantle_tx = create_test_mantle_tx(vec![Op::ChannelBlob(blob_op)]);
        let tx_hash = mantle_tx.hash().as_signing_bytes();
        let signature = signing_key.sign(&tx_hash);

        // Test too few proofs
        let result = SignedMantleTx::new(mantle_tx.clone(), vec![], dummy_zk_signature());
        assert!(matches!(
            result,
            Err(VerificationError::ProofCountMismatch {
                ops_count: 1,
                proofs_count: 0
            })
        ));

        // Test too many proofs
        let result = SignedMantleTx::new(
            mantle_tx,
            vec![
                Some(OpProof::Ed25519Sig(signature)),
                Some(OpProof::Ed25519Sig(signature)),
            ],
            dummy_zk_signature(),
        );
        assert!(matches!(
            result,
            Err(VerificationError::ProofCountMismatch {
                ops_count: 1,
                proofs_count: 2
            })
        ));
    }
}
