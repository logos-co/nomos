use core::fmt::Debug;

use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use bytes::Bytes;
use cryptarchia_engine::Slot;
use ed25519_dalek::Verifier as _;
use groth16::fr_to_bytes;

use crate::{
    codec::SerdeOp,
    header::{ContentId, Header, HeaderId},
    mantle::{AuthenticatedMantleTx, Transaction, TxHash},
    proofs::leader_proof::{Groth16LeaderProof, LeaderProof as _},
    utils::merkle,
};

const MAX_TRANSACTIONS: usize = 1024;

mod wire;

pub type BlockNumber = u64;
pub type SessionNumber = u64;

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to serialize: {0}")]
    Serialisation(#[from] crate::codec::Error),
    #[error("Signing error: {0}")]
    Signing(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub header: Header,
    pub transactions: References,
    pub signature: ed25519_dalek::Signature,
}

#[derive(Clone, Debug)]
pub struct References {
    pub service_reward: Option<TxHash>,
    pub mempool_transactions: Vec<TxHash>,
}

/// A block
#[derive(Clone, Debug)]
pub struct Block<Tx> {
    header: Header,
    signature: ed25519_dalek::Signature,
    service_reward: Option<Tx>,
    transactions: Vec<Tx>,
}

impl Proposal {
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    #[must_use]
    pub const fn references(&self) -> &References {
        &self.transactions
    }

    #[must_use]
    pub fn mempool_transactions(&self) -> &[TxHash] {
        &self.transactions.mempool_transactions
    }

    #[must_use]
    pub const fn signature(&self) -> &ed25519_dalek::Signature {
        &self.signature
    }
}

impl<Tx> Block<Tx> {
    pub fn create(
        parent_block: HeaderId,
        slot: Slot,
        proof_of_leadership: Groth16LeaderProof,
        transactions: Vec<Tx>,
        service_reward: Option<Tx>,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Result<Self, Error>
    where
        Tx: Transaction<Hash = TxHash>,
    {
        if transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::Signing(format!(
                "Too many transactions (max {MAX_TRANSACTIONS})"
            )));
        }

        let block_root =
            Self::calculate_content_id_from_parts(&transactions, service_reward.as_ref());

        let header = Header::new(parent_block, block_root, slot, proof_of_leadership);

        let signature = header.sign(signing_key)?;

        Ok(Self {
            header,
            signature,
            service_reward,
            transactions,
        })
    }

    pub fn recover(
        header: Header,
        transactions: Vec<Tx>,
        service_reward: Option<Tx>,
        signature: ed25519_dalek::Signature,
    ) -> Result<Self, Error>
    where
        Tx: Transaction<Hash = TxHash>,
    {
        if !header.is_valid_bedrock_version() {
            return Err(Error::Signing("Invalid header version".to_owned()));
        }

        if transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::Signing(format!(
                "Too many transactions (max {MAX_TRANSACTIONS})"
            )));
        }

        let calculated_content_id =
            Self::calculate_content_id_from_parts(&transactions, service_reward.as_ref());
        if header.block_root() != &calculated_content_id {
            return Err(Error::Signing(
                "Block root mismatch with transactions".to_owned(),
            ));
        }

        let leader_public_key = header.leader_proof().leader_key();
        let header_bytes = <Header as SerdeOp>::serialize(&header)?;

        leader_public_key
            .verify(&header_bytes, &signature)
            .map_err(|e| Error::Signing(format!("Invalid signature: {e}")))?;

        Ok(Self {
            header,
            signature,
            service_reward,
            transactions,
        })
    }

    fn calculate_content_id_from_parts(
        transactions: &[Tx],
        service_reward: Option<&Tx>,
    ) -> ContentId
    where
        Tx: Transaction<Hash = TxHash>,
    {
        let mut all_transactions = Vec::new();
        if let Some(reward) = service_reward {
            all_transactions.push(reward);
        }
        all_transactions.extend(transactions.iter());

        if all_transactions.is_empty() {
            return ContentId::from([0; 32]);
        }

        let merkle_leaves: Vec<merkle::MerkleNode<TxHash>> = all_transactions
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                if i % 2 == 0 {
                    merkle::MerkleNode::Left(tx.hash())
                } else {
                    merkle::MerkleNode::Right(tx.hash())
                }
            })
            .collect();

        let root_hash = merkle::calculate_merkle_root(&merkle_leaves);
        root_hash.map_or_else(
            || ContentId::from([0; 32]),
            |hash| ContentId::from(fr_to_bytes(&hash)),
        )
    }

    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    #[must_use]
    pub fn transactions(&self) -> impl ExactSizeIterator<Item = &Tx> + '_ {
        self.transactions.iter()
    }

    #[must_use]
    pub fn into_transactions(self) -> Vec<Tx> {
        self.transactions
    }

    #[must_use]
    pub const fn service_reward(&self) -> &Option<Tx> {
        &self.service_reward
    }

    #[must_use]
    pub const fn signature(&self) -> &ed25519_dalek::Signature {
        &self.signature
    }

    pub fn to_proposal(&self) -> Proposal
    where
        Tx: AuthenticatedMantleTx + Clone,
    {
        let mempool_transactions: Vec<TxHash> =
            self.transactions.iter().map(Transaction::hash).collect();

        let service_reward_hash = self.service_reward.as_ref().map(Transaction::hash);

        let transactions = References {
            service_reward: service_reward_hash,
            mempool_transactions,
        };

        Proposal {
            header: self.header.clone(),
            transactions,
            signature: self.signature,
        }
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Bytes> for Block<Tx> {
    type Error = crate::codec::Error;

    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        <Self as SerdeOp>::deserialize(&bytes)
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Block<Tx>> for Bytes {
    type Error = crate::codec::Error;

    fn try_from(block: Block<Tx>) -> Result<Self, Self::Error> {
        <Block<Tx> as SerdeOp>::serialize(&block)
    }
}

#[cfg(test)]
mod tests {
    use std::iter;

    use ed25519_dalek::SigningKey;
    use groth16::Fr;
    use num_bigint::BigUint;

    use super::*;
    use crate::{
        mantle::{
            ledger::{Note, Tx, Utxo},
            ops::leader_claim::VoucherCm,
        },
        proofs::leader_proof::{LeaderPrivate, LeaderPublic},
        utils::merkle::MerkleNode,
    };

    pub fn create_proof() -> Groth16LeaderProof {
        let public_inputs = LeaderPublic::new(Fr::from(1), Fr::from(2), Fr::from(3), 0, 1000);

        let utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: Note::new(100, Fr::from(5).into()),
        };

        let aged_path = vec![MerkleNode::Right(Fr::from(0u8))];
        let latest_path = vec![MerkleNode::Left(Fr::from(0u8))];

        let signing_key = SigningKey::from_bytes(&[0; 32]);
        let verifying_key = signing_key.verifying_key();

        let private_inputs = LeaderPrivate::new(
            public_inputs,
            utxo,
            &aged_path,
            &latest_path,
            Fr::from(6),
            0,
            &verifying_key,
        );
        Groth16LeaderProof::prove(&private_inputs, VoucherCm::default())
            .expect("Proof generation should succeed")
    }

    fn create_transactions(count: usize) -> Vec<Tx> {
        iter::repeat_with(|| Tx {
            inputs: vec![],
            outputs: vec![],
        })
        .take(count)
        .collect()
    }

    #[test]
    fn test_block_signature_validation() {
        let parent_block = [0u8; 32].into();
        let slot = Slot::from(42u64);
        let proof_of_leadership = create_proof();
        let transactions: Vec<Tx> = vec![];
        let service_reward = None;

        let valid_signing_key = SigningKey::from_bytes(&[0; 32]);
        let valid_block = Block::create(
            parent_block,
            slot,
            proof_of_leadership,
            transactions.clone(),
            service_reward.clone(),
            &valid_signing_key,
        )
        .expect("Valid block should be created");

        let header = valid_block.header().clone();
        let valid_signature = *valid_block.signature();

        let _recovered_block = Block::recover(
            header.clone(),
            transactions.clone(),
            service_reward.clone(),
            valid_signature,
        )
        .expect("Should recover block with valid signature");

        let wrong_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let invalid_signature = header
            .sign(&wrong_signing_key)
            .expect("Signing should work");

        let invalid_block_result =
            Block::recover(header, transactions, service_reward, invalid_signature);

        assert!(
            invalid_block_result.is_err(),
            "Should not recover block with invalid signature"
        );
    }

    #[test]
    fn test_block_transaction_count_validation() {
        let parent_block = [0u8; 32].into();
        let slot = Slot::from(42u64);
        let proof_of_leadership = create_proof();
        let service_reward = None;
        let signing_key = SigningKey::from_bytes(&[0; 32]);

        let _valid_block: Block<Tx> = Block::create(
            parent_block,
            slot,
            proof_of_leadership.clone(),
            vec![],
            service_reward.clone(),
            &signing_key,
        )
        .expect("Valid block should be created");

        let invalid_block_result = Block::create(
            parent_block,
            slot,
            proof_of_leadership,
            create_transactions(MAX_TRANSACTIONS + 1),
            service_reward,
            &signing_key,
        );

        assert!(invalid_block_result.is_err());
        let error_msg = invalid_block_result.unwrap_err().to_string();

        assert!(error_msg.contains("Too many transactions"));
        assert!(error_msg.contains(&MAX_TRANSACTIONS.to_string()));
    }
}
