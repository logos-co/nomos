use core::fmt::Debug;

use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use bytes::Bytes;
use cryptarchia_engine::Slot;
use ed25519_dalek::Verifier as _;

use crate::{
    codec::{DeserializeOp as _, SerializeOp as _},
    header::{ContentId, Header, HeaderId},
    mantle::{Transaction, TxHash},
    proofs::leader_proof::{Groth16LeaderProof, LeaderProof as _},
    utils::merkle,
};

pub const MAX_TRANSACTIONS: usize = 1024;

pub type BlockNumber = u64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to serialize: {0}")]
    Serialisation(#[from] crate::codec::Error),
    #[error("Signature error: {0}")]
    Signature(Box<ed25519_dalek::SignatureError>),
    #[error("Too many transactions: {count} exceeds maximum of {max}")]
    TooManyTxs { count: usize, max: usize },
    #[error("Block root mismatch: calculated content does not match header")]
    BlockRootMismatch,
    #[error("Signing key does not match the leader key in proof of leadership")]
    KeyMismatch,
    #[error("Validation error: {0}")]
    Validation(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub header: Header,
    pub references: References,
    pub signature: ed25519_dalek::Signature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct References {
    pub service_reward: Option<TxHash>,
    pub mempool_transactions: Vec<TxHash>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        &self.references
    }

    #[must_use]
    pub fn mempool_transactions(&self) -> &[TxHash] {
        &self.references.mempool_transactions
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
        let expected_public_key = proof_of_leadership.leader_key();
        let actual_public_key = signing_key.verifying_key();
        if expected_public_key != &actual_public_key {
            return Err(Error::KeyMismatch);
        }

        if transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::TooManyTxs {
                count: transactions.len(),
                max: MAX_TRANSACTIONS,
            });
        }

        let block_root = Self::calculate_content_id(&transactions, service_reward.as_ref());

        let header = Header::new(parent_block, block_root, slot, proof_of_leadership);

        let signature = header.sign(signing_key)?;

        Ok(Self {
            header,
            signature,
            service_reward,
            transactions,
        })
    }

    pub fn reconstruct(
        header: Header,
        transactions: Vec<Tx>,
        service_reward: Option<Tx>,
        signature: ed25519_dalek::Signature,
    ) -> Result<Self, Error>
    where
        Tx: Transaction<Hash = TxHash>,
    {
        if transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::TooManyTxs {
                count: transactions.len(),
                max: MAX_TRANSACTIONS,
            });
        }

        let calculated_content_id =
            Self::calculate_content_id(&transactions, service_reward.as_ref());
        if header.block_root() != &calculated_content_id {
            return Err(Error::BlockRootMismatch);
        }

        let leader_public_key = header.leader_proof().leader_key();
        let header_bytes = header.to_bytes()?;

        leader_public_key
            .verify(&header_bytes, &signature)
            .map_err(|e| Error::Signature(Box::new(e)))?;

        Ok(Self {
            header,
            signature,
            service_reward,
            transactions,
        })
    }

    fn calculate_content_id(transactions: &[Tx], service_reward: Option<&Tx>) -> ContentId
    where
        Tx: Transaction<Hash = TxHash>,
    {
        let tx_hashes: Vec<TxHash> = service_reward
            .into_iter()
            .chain(transactions.iter())
            .map(Transaction::hash)
            .collect();

        let root_hash = merkle::calculate_merkle_root(&tx_hashes, None);
        ContentId::from(root_hash)
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
    pub const fn transactions_vec(&self) -> &Vec<Tx> {
        &self.transactions
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

    pub fn to_proposal(self) -> Proposal
    where
        Tx: Transaction<Hash = TxHash>,
    {
        let mempool_transactions: Vec<TxHash> =
            self.transactions.iter().map(Transaction::hash).collect();

        let service_reward_hash = self.service_reward.as_ref().map(Transaction::hash);

        let references = References {
            service_reward: service_reward_hash,
            mempool_transactions,
        };

        Proposal {
            header: self.header,
            references,
            signature: self.signature,
        }
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Bytes> for Block<Tx> {
    type Error = crate::codec::Error;

    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        Self::from_bytes(&bytes)
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Block<Tx>> for Bytes {
    type Error = crate::codec::Error;

    fn try_from(block: Block<Tx>) -> Result<Self, Self::Error> {
        block.to_bytes()
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
        Groth16LeaderProof::prove(private_inputs, VoucherCm::default())
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

        let _reconstructed_block = Block::reconstruct(
            header.clone(),
            transactions.clone(),
            service_reward.clone(),
            valid_signature,
        )
        .expect("Should reconstruct block with valid signature");

        let wrong_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let invalid_signature = header
            .sign(&wrong_signing_key)
            .expect("Signing should work");

        let invalid_block_result =
            Block::reconstruct(header, transactions, service_reward, invalid_signature);

        assert!(
            invalid_block_result.is_err(),
            "Should not reconstruct block with invalid signature"
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
        let error = invalid_block_result.unwrap_err();

        let expected_count = MAX_TRANSACTIONS + 1;
        assert!(
            matches!(error, Error::TooManyTxs { count, max } if count == expected_count && max == MAX_TRANSACTIONS)
        );
    }
}
