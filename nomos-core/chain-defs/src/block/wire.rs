use groth16::{Fr, serde::serde_fr};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    block::{Block, References},
    header::Header,
    mantle::TxHash,
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
struct WireReferences {
    service_reward: Option<TxHash>,
    mempool_transactions: Vec<TxHash>,
}

impl Serialize for References {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = WireReferences {
            service_reward: self.service_reward,
            mempool_transactions: self.mempool_transactions.clone(),
        };

        Serialize::serialize(&wire, serializer)
    }
}

impl<'de> Deserialize<'de> for References {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire: WireReferences = <WireReferences as Deserialize>::deserialize(deserializer)?;

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

    use super::Fr;
    use crate::{
        block::{Block, References, tests::create_proof},
        codec::SerdeOp,
        mantle::{SignedMantleTx, TxHash, ledger::Tx},
    };

    #[test]
    fn test_transactions_serialization() {
        let service_reward = Some(TxHash::from(Fr::from(999u64)));
        let mempool_transactions = vec![
            TxHash::from(Fr::from(123u64)),
            TxHash::from(Fr::from(456u64)),
            TxHash::from(Fr::from(789u64)),
        ];

        let transactions = References {
            service_reward,
            mempool_transactions,
        };

        let wire_bytes = <References as SerdeOp>::serialize(&transactions)
            .expect("Failed to serialize with wire");

        let deserialized: References = <References as SerdeOp>::deserialize(&wire_bytes)
            .expect("Failed to deserialize with wire");

        assert_eq!(transactions.service_reward, deserialized.service_reward);
        assert_eq!(
            transactions.mempool_transactions,
            deserialized.mempool_transactions
        );

        assert_eq!(
            deserialized.service_reward,
            Some(TxHash::from(Fr::from(999u64)))
        );
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
        let parent_block = [0u8; 32].into();
        let slot = Slot::from(42u64);
        let proof_of_leadership = create_proof();

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
        let mantle_transactions = vec![service_reward.clone()];

        let block: Block<SignedMantleTx> = Block::create(
            parent_block,
            slot,
            proof_of_leadership,
            mantle_transactions,
            Some(service_reward),
            &signing_key,
        )
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
