use groth16::Fr;
use poseidon2::Digest;
use serde::{Deserialize, Serialize};

use super::OpProof;
use crate::{
    crypto::ZkHasher,
    mantle::{
        MantleTx, Transaction, TransactionHasher, TxHash,
        gas::{Gas, GasConstants, GasCost},
        ops::{
            Op,
            channel::{ChannelId, MsgId, inscribe::InscriptionOp},
        },
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenesisTx {
    mantle_tx: MantleTx,
    op_proofs: Vec<Option<OpProof>>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Genesis transaction must have gas price of zero")]
    InvalidGenesisGasPrice,
    #[error("Genesis transaction should not have any inputs")]
    UnepectedInput,
    #[error("Genesis block cannot contain this op: {0:?}")]
    UnsupportedGenesisOp(Vec<Op>),
    #[error("Expected exactly one inscription in genesis block")]
    MissingInscription,
    #[error("Invalid genesis inscription: {0:?}")]
    InvalidInscription(Box<Op>),
    #[error("Unequal number of ops ({num_ops}) and proofs ({num_proofs})")]
    UnequalNumberOfProofs { num_ops: usize, num_proofs: usize },
}

impl GenesisTx {
    pub fn from_tx(mantle_tx: MantleTx) -> Result<Self, Error> {
        // Genesis transactions must have gas prices of zero
        if mantle_tx.execution_gas_price != 0 || mantle_tx.storage_gas_price != 0 {
            return Err(Error::InvalidGenesisGasPrice);
        }

        // Genesis transactions should not have any inputs
        if !mantle_tx.ledger_tx.inputs.is_empty() {
            return Err(Error::UnepectedInput);
        }

        // Genesis transactions must contain exactly one inscription as the first op
        // and then may contain other SDP declarations
        let mut ops = mantle_tx.ops.iter();
        match ops.next() {
            Some(Op::ChannelInscribe(op)) => valid_cryptarchia_inscription(op)?,
            _ => return Err(Error::MissingInscription),
        }

        let unsupported_ops = ops
            .filter(|op| !matches!(op, Op::SDPDeclare(_)))
            .cloned()
            .collect::<Vec<_>>();
        if !unsupported_ops.is_empty() {
            return Err(Error::UnsupportedGenesisOp(unsupported_ops));
        }

        Ok(Self {
            mantle_tx,
            op_proofs: vec![],
        })
    }

    pub fn with_proofs(mut self, op_proofs: Vec<Option<OpProof>>) -> Result<Self, Error> {
        let num_ops = self.mantle_tx.ops.len();
        let num_proofs = op_proofs.len();

        if num_ops != num_proofs {
            return Err(Error::UnequalNumberOfProofs {
                num_ops,
                num_proofs,
            });
        }

        self.op_proofs = op_proofs;
        Ok(self)
    }
}

fn valid_cryptarchia_inscription(inscription: &InscriptionOp) -> Result<(), Error> {
    if inscription.parent != MsgId::root() {
        return Err(Error::InvalidInscription(Box::new(Op::ChannelInscribe(
            inscription.clone(),
        ))));
    }

    if inscription.channel_id != ChannelId::from([0; 32]) {
        return Err(Error::InvalidInscription(Box::new(Op::ChannelInscribe(
            inscription.clone(),
        ))));
    }

    if inscription.signer.as_bytes() != &[0; 32] {
        return Err(Error::InvalidInscription(Box::new(Op::ChannelInscribe(
            inscription.clone(),
        ))));
    }

    Ok(())
}

impl Transaction for GenesisTx {
    const HASHER: TransactionHasher<Self> =
        |tx| <ZkHasher as Digest>::digest(&tx.as_signing_frs()).into();
    type Hash = TxHash;
    fn as_signing_frs(&self) -> Vec<Fr> {
        self.mantle_tx.as_signing_frs()
    }
}

impl GasCost for GenesisTx {
    fn gas_cost<Constants: GasConstants>(&self) -> Gas {
        // Genesis transactions have zero gas cost as per spec
        0
    }
}

impl crate::mantle::GenesisTx for GenesisTx {
    fn genesis_inscription(&self) -> &InscriptionOp {
        // Safe to unwrap because we validated this in from_tx
        match &self.mantle_tx.ops[0] {
            Op::ChannelInscribe(op) => op,
            _ => unreachable!("GenesisTx always has a valid inscription as first op"),
        }
    }

    fn op_proofs(&self) -> &Vec<Option<OpProof>> {
        &self.op_proofs
    }

    fn mantle_tx(&self) -> &MantleTx {
        &self.mantle_tx
    }
}

impl<'de> Deserialize<'de> for GenesisTx {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct UncheckedGenesisTx {
            mantle_tx: MantleTx,
            op_proofs: Vec<Option<OpProof>>,
        }

        let genesis_tx = UncheckedGenesisTx::deserialize(deserializer)?;

        Self::from_tx(genesis_tx.mantle_tx)
            .map_err(serde::de::Error::custom)?
            .with_proofs(genesis_tx.op_proofs)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::VerifyingKey;
    use num_bigint::BigUint;

    use super::*;
    use crate::{
        mantle::{
            keys::PublicKey,
            ledger::{Note, Tx as LedgerTx, Utxo, Value},
            ops::{channel::blob::BlobOp, sdp::SDPDeclareOp},
        },
        sdp::{ProviderId, ServiceType, ZkPublicKey},
    };

    fn inscription_op(channel_id: ChannelId, parent: MsgId, signer: VerifyingKey) -> InscriptionOp {
        InscriptionOp {
            channel_id,
            inscription: vec![1, 2, 3, 4],
            parent,
            signer,
        }
    }

    fn sdp_declare_op(
        utxo_to_use: Utxo,
        zk_id_value: u8,
        verifying_key: VerifyingKey,
    ) -> SDPDeclareOp {
        SDPDeclareOp {
            service_type: ServiceType::BlendNetwork,
            locked_note_id: utxo_to_use.id(),
            zk_id: ZkPublicKey(BigUint::from(zk_id_value).into()),
            provider_id: ProviderId(verifying_key),
            locators: [].into(),
        }
    }

    fn blob_op(channel_id: ChannelId, verifying_key: VerifyingKey) -> BlobOp {
        BlobOp {
            channel: channel_id,
            blob: [42; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::root(),
            signer: verifying_key,
        }
    }

    // Helper function to create a test note
    fn create_test_note(value: Value) -> Note {
        Note::new(value, PublicKey::from(BigUint::from(123u64)))
    }

    // Helper function to create a basic signed transaction
    // Genesis transactions don't need verified proofs for Blob/Inscription ops
    fn create_tx(ops: Vec<Op>) -> MantleTx {
        let ledger_tx = LedgerTx::new(vec![], vec![create_test_note(1000)]);
        MantleTx {
            ops,
            ledger_tx,
            execution_gas_price: 0,
            storage_gas_price: 0,
        }
    }

    #[test]
    fn test_inscription_fields() {
        // check inscription with channel id [1; 32] fails
        let tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([1; 32]),
            MsgId::root(),
            VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        ))]);
        assert!(matches!(
            GenesisTx::from_tx(tx),
            Err(Error::InvalidInscription(_))
        ));

        // check inscription with non-root parent fails
        let tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([0; 32]),
            MsgId::from([1; 32]),
            VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        ))]);
        assert!(matches!(
            GenesisTx::from_tx(tx),
            Err(Error::InvalidInscription(_))
        ));

        // check inscription with non-zero signer fails
        let tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([0; 32]),
            MsgId::root(),
            VerifyingKey::from_bytes(&[1; 32]).unwrap(),
        ))]);
        assert!(matches!(
            GenesisTx::from_tx(tx),
            Err(Error::InvalidInscription(_))
        ));

        // check valid inscription passes
        let tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([0; 32]),
            MsgId::root(),
            VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        ))]);
        assert!(GenesisTx::from_tx(tx).is_ok());
    }

    #[test]
    fn test_genesis_inscription_ops() {
        let inscription_op = || {
            inscription_op(
                ChannelId::from([0; 32]),
                MsgId::root(),
                VerifyingKey::from_bytes(&[0; 32]).unwrap(),
            )
        };
        let blob_op = || {
            blob_op(
                ChannelId::from([0; 32]),
                VerifyingKey::from_bytes(&[0; 32]).unwrap(),
            )
        };

        // Test cases: (operations, expected_error)
        let test_cases = [
            // no inscription -> error
            (vec![], Some(Error::MissingInscription)),
            // one inscription -> ok
            (vec![Op::ChannelInscribe(inscription_op())], None),
            // two inscriptions -> error
            (
                vec![
                    Op::ChannelInscribe(inscription_op()),
                    Op::ChannelInscribe(inscription_op()),
                ],
                Some(Error::UnsupportedGenesisOp(vec![Op::ChannelInscribe(
                    inscription_op(),
                )])),
            ),
            // Invalid non-SDP combinations
            (
                vec![
                    Op::ChannelInscribe(inscription_op()),
                    Op::ChannelBlob(blob_op()),
                ],
                Some(Error::UnsupportedGenesisOp(vec![
                    Op::ChannelBlob(blob_op()),
                ])),
            ),
        ];

        // Execute all test cases
        for (ops, expected_err) in test_cases {
            let tx = create_tx(ops);
            let result = GenesisTx::from_tx(tx);
            match expected_err {
                Some(expected) => assert_eq!(result, Err(expected)),
                None => assert!(result.is_ok()),
            }
        }
    }

    #[test]
    fn test_genesis_sdp_ops() {
        let inscription_op = || {
            inscription_op(
                ChannelId::from([0; 32]),
                MsgId::root(),
                VerifyingKey::from_bytes(&[0; 32]).unwrap(),
            )
        };
        let verifying_key = VerifyingKey::from_bytes(&[0; 32]).unwrap();
        let utxo1 = Utxo::new(TxHash::from(Fr::from(0u64)), 0, create_test_note(1000));
        let utxo2 = Utxo::new(TxHash::from(Fr::from(1u64)), 1, create_test_note(2000));
        let sdp_declare_op_helper = |utxo_to_use: Utxo, zk_id_value: u8| {
            sdp_declare_op(utxo_to_use, zk_id_value, verifying_key)
        };
        let blob_op = || {
            blob_op(
                ChannelId::from([0; 32]),
                VerifyingKey::from_bytes(&[0; 32]).unwrap(),
            )
        };

        // Test cases: (operations, expected_error)
        let test_cases = [
            // SDP without inscription
            (
                vec![Op::SDPDeclare(sdp_declare_op_helper(utxo1, 0))],
                Some(Error::MissingInscription),
            ),
            // Valid SDP combinations
            (
                vec![
                    Op::ChannelInscribe(inscription_op()),
                    Op::SDPDeclare(sdp_declare_op_helper(utxo1, 0)),
                ],
                None,
            ),
            (
                vec![
                    Op::ChannelInscribe(inscription_op()),
                    Op::SDPDeclare(sdp_declare_op_helper(utxo1, 0)),
                    Op::SDPDeclare(sdp_declare_op_helper(utxo2, 1)),
                ],
                None,
            ),
            // Invalid mixed combinations
            (
                vec![
                    Op::ChannelInscribe(inscription_op()),
                    Op::SDPDeclare(sdp_declare_op_helper(utxo1, 0)),
                    Op::ChannelBlob(blob_op()),
                ],
                Some(Error::UnsupportedGenesisOp(vec![
                    Op::ChannelBlob(blob_op()),
                ])),
            ),
        ];

        // Execute all test cases
        for (ops, expected_err) in test_cases {
            let tx = create_tx(ops);
            let result = GenesisTx::from_tx(tx);
            match expected_err {
                Some(expected) => assert_eq!(result, Err(expected)),
                None => assert!(result.is_ok()),
            }
        }
    }

    #[test]
    fn test_genesis_fees() {
        // Should succeed with zero gas prices
        let mut tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([0; 32]),
            MsgId::root(),
            VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        ))]);
        assert!(GenesisTx::from_tx(tx.clone()).is_ok());

        // Test with non-zero execution gas price
        tx.execution_gas_price = 1;
        let result = GenesisTx::from_tx(tx.clone());
        assert_eq!(result, Err(Error::InvalidGenesisGasPrice));

        // test with non-zero storage gas price
        tx.storage_gas_price = 1;
        tx.execution_gas_price = 0;
        let result = GenesisTx::from_tx(tx.clone());
        assert_eq!(result, Err(Error::InvalidGenesisGasPrice));

        // test with both gas prices non-zero
        tx.storage_gas_price = 1;
        tx.execution_gas_price = 1;
        let result = GenesisTx::from_tx(tx);
        assert_eq!(result, Err(Error::InvalidGenesisGasPrice));
    }

    #[test]
    fn test_genesis_tx_serde() {
        // Create a genesis transaction with inscription (no signature proof required)
        let tx = create_tx(vec![Op::ChannelInscribe(inscription_op(
            ChannelId::from([0; 32]),
            MsgId::root(),
            VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        ))]);
        let genesis_tx = GenesisTx::from_tx(tx).expect("Valid genesis transaction");

        // Serialize to JSON
        let json_str = serde_json::to_string(&genesis_tx).expect("Serialization should succeed");

        // Deserialize from JSON fails because of missing proofs.
        let result = serde_json::from_str::<GenesisTx>(&json_str)
            .unwrap_err()
            .to_string();

        assert_eq!(
            result,
            Error::UnequalNumberOfProofs {
                num_ops: 1,
                num_proofs: 0
            }
            .to_string()
        );

        // Serialize and deserialize with proofs.
        let genesis_tx = genesis_tx.with_proofs(vec![None]).unwrap();
        let json_str = serde_json::to_string(&genesis_tx).expect("Serialization should succeed");
        let deserialized: GenesisTx = serde_json::from_str(&json_str).unwrap();

        // Verify they're equal
        assert_eq!(genesis_tx, deserialized);
    }
}
