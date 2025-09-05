use groth16::{Fr, serde::serde_fr};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    block::{Block, References},
    header::Header,
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
    service_reward: Option<SerializableFr>,
    mempool_transactions: Vec<SerializableFr>,
}

impl Serialize for References {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let service_reward = self.service_reward.map(SerializableFr::from);
        let mempool_transactions = self
            .mempool_transactions
            .iter()
            .map(|fr| SerializableFr::from(*fr))
            .collect();

        let wire = WireReferences {
            service_reward,
            mempool_transactions,
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

        let service_reward = wire.service_reward.map(Fr::from);
        let mempool_transactions = wire
            .mempool_transactions
            .into_iter()
            .map(Fr::from)
            .collect();

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
    service_reward: Option<SerializableFr>,
    transactions: Vec<Tx>,
}

impl<Tx: Serialize + Clone> Serialize for Block<Tx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let service_reward = self.service_reward.map(SerializableFr::from);

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

        let service_reward = wire.service_reward.map(Fr::from);

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
    use ed25519_dalek::SigningKey;
    use num_bigint::BigUint;

    use super::Fr;
    use crate::{
        block::{Block, References},
        codec::SerdeOp,
        mantle::{
            ledger::{Note, Utxo},
            ops::leader_claim::VoucherCm,
        },
        proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
        utils::merkle::MerkleNode,
    };

    #[test]
    fn test_references_serialization() {
        let service_reward = Some(Fr::from(42u64));
        let mempool_transactions = vec![Fr::from(123u64), Fr::from(456u64), Fr::from(789u64)];

        let references = References {
            service_reward,
            mempool_transactions,
        };

        let wire_bytes =
            <References as SerdeOp>::serialize(&references).expect("Failed to serialize with wire");
        let deserialized: References = <References as SerdeOp>::deserialize(&wire_bytes)
            .expect("Failed to deserialize with wire");

        assert_eq!(references.service_reward, deserialized.service_reward);
        assert_eq!(
            references.mempool_transactions,
            deserialized.mempool_transactions
        );

        assert_eq!(deserialized.service_reward.unwrap(), Fr::from(42u64));
        assert_eq!(deserialized.mempool_transactions.len(), 3);
        assert_eq!(deserialized.mempool_transactions[0], Fr::from(123u64));
        assert_eq!(deserialized.mempool_transactions[1], Fr::from(456u64));
        assert_eq!(deserialized.mempool_transactions[2], Fr::from(789u64));
    }

    // TODO: probably should have some utility for this in workspace
    fn make_test_proof() -> Groth16LeaderProof {
        let public_inputs = LeaderPublic::new(
            Fr::from(1), // aged root
            Fr::from(2), // latest root
            Fr::from(3), // epoch nonce
            0,           // slot
            1000,        // total stake
        );
        let utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: Note::new(100, Fr::from(5).into()),
        };
        let aged_path = vec![MerkleNode::Right(Fr::from(0u8))];
        let latest_path = vec![MerkleNode::Left(Fr::from(0u8))];
        let private_inputs = LeaderPrivate::new(
            public_inputs,
            utxo,
            &aged_path,
            &latest_path,
            Fr::from(6), // slot secret
            0,           // starting slot
            &ed25519_dalek::VerifyingKey::from_bytes(&[0; 32]).unwrap(),
        );
        Groth16LeaderProof::prove(&private_inputs, VoucherCm::default())
            .expect("Proof generation should succeed")
    }

    #[test]
    fn test_block_serialization() {
        use cryptarchia_engine::Slot;

        use crate::header::{ContentId, Header};

        let header = Header::new(
            [0u8; 32].into(),
            ContentId::from([1u8; 32]),
            Slot::from(42u64),
            make_test_proof(),
        );

        let transactions = vec!["tx1".to_owned(), "tx2".to_owned()];
        let service_reward = Fr::from(123u64);

        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let signature = header
            .sign(&signing_key)
            .expect("Header signing should work in test");

        let block = Block::new(header, transactions, Some(service_reward), signature);

        let wire_bytes =
            <Block<String> as SerdeOp>::serialize(&block).expect("Failed to serialize block");
        let deserialized: Block<String> = <Block<String> as SerdeOp>::deserialize(&wire_bytes)
            .expect("Failed to deserialize block");

        assert_eq!(block.service_reward, deserialized.service_reward);
        assert_eq!(block.transactions, deserialized.transactions);
        assert_eq!(deserialized.service_reward.unwrap(), Fr::from(123u64));
    }
}
