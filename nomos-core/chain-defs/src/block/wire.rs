use groth16::{Fr, serde::serde_fr};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    block::{Block, Transactions},
    header::Header,
    mantle::{AuthenticatedMantleTx, TxHash},
};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct SerializableFr(#[serde(with = "serde_fr")] Fr);

impl From<Fr> for SerializableFr {
    fn from(fr: Fr) -> Self {
        Self(fr)
    }
}

impl From<SerializableFr> for Fr {
    fn from(serializable: SerializableFr) -> Self {
        serializable.0
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WireReferences<Tx> {
    service_reward: Option<Tx>,
    mempool_transactions: Vec<TxHash>,
}

impl<Tx: Serialize + Clone + AuthenticatedMantleTx> Serialize for Transactions<Tx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = WireReferences {
            service_reward: self.service_reward.clone(),
            mempool_transactions: self.mempool_transactions.clone(),
        };

        Serialize::serialize(&wire, serializer)
    }
}

impl<'de, Tx: Deserialize<'de> + AuthenticatedMantleTx + Clone> Deserialize<'de>
    for Transactions<Tx>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire: WireReferences<Tx> =
            <WireReferences<Tx> as Deserialize>::deserialize(deserializer)?;

        let service_reward = wire.service_reward;
        let mempool_transactions = wire.mempool_transactions;

        Ok(Self {
            service_reward,
            mempool_transactions,
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WireBlock<Tx> {
    header: Header,
    signature: ed25519_dalek::Signature,
    service_reward: Option<Tx>,
    transactions: Vec<Tx>,
}

impl<Tx: Serialize + Clone> Serialize for Block<Tx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let service_reward = self.service_reward.clone();

        let wire = WireBlock {
            header: self.header.clone(),
            signature: self.signature,
            service_reward,
            transactions: self.transactions.clone(),
        };

        Serialize::serialize(&wire, serializer)
    }
}

impl<'de, Tx: Deserialize<'de>> Deserialize<'de> for Block<Tx> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire: WireBlock<Tx> = <WireBlock<Tx> as Deserialize>::deserialize(deserializer)?;

        let service_reward = wire.service_reward;

        Ok(Self {
            header: wire.header,
            signature: wire.signature,
            service_reward,
            transactions: wire.transactions,
        })
    }
}

#[cfg(test)]
mod tests {
    use cryptarchia_engine::Slot;
    use ed25519_dalek::SigningKey;

    use super::{super::BuilderWithSignature, Fr};
    use crate::{
        block::{Block, Transactions, tests::create_proof},
        codec::SerdeOp,
        header::{ContentId, Header},
        mantle::{SignedMantleTx, TxHash, ledger::Tx},
    };

    #[test]
    fn test_transactions_serialization() {
        let service_reward = Some(SignedMantleTx {
            mantle_tx: crate::mantle::MantleTx {
                ops: vec![],
                ledger_tx: Tx {
                    inputs: vec![],
                    outputs: vec![],
                },
                execution_gas_price: 0,
                storage_gas_price: 0,
            },
            ops_proofs: vec![],
            ledger_tx_proof: crate::proofs::zksig::DummyZkSignature::prove(
                crate::proofs::zksig::ZkSignaturePublic {
                    msg_hash: Fr::from(0u64),
                    pks: vec![],
                },
            ),
        });
        let mempool_transactions = vec![
            TxHash::from(Fr::from(123u64)),
            TxHash::from(Fr::from(456u64)),
            TxHash::from(Fr::from(789u64)),
        ];

        let transactions = Transactions {
            service_reward,
            mempool_transactions,
        };

        let wire_bytes = <Transactions<SignedMantleTx> as SerdeOp>::serialize(&transactions)
            .expect("Failed to serialize with wire");

        let deserialized: Transactions<SignedMantleTx> =
            <Transactions<SignedMantleTx> as SerdeOp>::deserialize(&wire_bytes)
                .expect("Failed to deserialize with wire");

        assert_eq!(transactions.service_reward, deserialized.service_reward);
        assert_eq!(
            transactions.mempool_transactions,
            deserialized.mempool_transactions
        );

        assert!(deserialized.service_reward.is_some());
        assert_eq!(deserialized.mempool_transactions.len(), 3);
        assert_eq!(
            deserialized.mempool_transactions[0],
            TxHash::from(Fr::from(123u64))
        );
        assert_eq!(
            deserialized.mempool_transactions[1],
            TxHash::from(Fr::from(456u64))
        );
        assert_eq!(
            deserialized.mempool_transactions[2],
            TxHash::from(Fr::from(789u64))
        );
    }

    #[test]
    fn test_block_serialization() {
        let header = Header::new(
            [0u8; 32].into(),
            ContentId::from([1u8; 32]),
            Slot::from(42u64),
            create_proof(),
        );

        let service_reward = SignedMantleTx {
            mantle_tx: crate::mantle::MantleTx {
                ops: vec![],
                ledger_tx: Tx {
                    inputs: vec![],
                    outputs: vec![],
                },
                execution_gas_price: 0,
                storage_gas_price: 0,
            },
            ops_proofs: vec![],
            ledger_tx_proof: crate::proofs::zksig::DummyZkSignature::prove(
                crate::proofs::zksig::ZkSignaturePublic {
                    msg_hash: Fr::from(0u64),
                    pks: vec![],
                },
            ),
        };

        let signing_key = SigningKey::from_bytes(&[0; 32]);
        let signature = header
            .sign(&signing_key)
            .expect("Header signing should work in test");

        let mantle_transactions = vec![service_reward.clone()];
        let block: Block<SignedMantleTx> = Block::builder()
            .header(header)
            .and_then(|b| b.transactions(mantle_transactions))
            .map(|b| b.service_reward(Some(service_reward)))
            .and_then(|b| b.signature(signature))
            .and_then(BuilderWithSignature::build)
            .expect("Valid block should be created");

        let wire_bytes = <Block<SignedMantleTx> as SerdeOp>::serialize(&block)
            .expect("Failed to serialize block");

        let deserialized: Block<SignedMantleTx> =
            <Block<SignedMantleTx> as SerdeOp>::deserialize(&wire_bytes)
                .expect("Failed to deserialize block");

        assert_eq!(block.service_reward, deserialized.service_reward);
        assert_eq!(block.transactions, deserialized.transactions);
    }
}
