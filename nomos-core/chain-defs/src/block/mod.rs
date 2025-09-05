use core::fmt::Debug;

use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use bytes::Bytes;
use ed25519_dalek::Verifier as _;
use groth16::Fr;

use crate::{
    codec::SerdeOp,
    header::Header,
    mantle::{AuthenticatedMantleTx, Transaction, TxHash},
    proofs::leader_proof::LeaderProof as _,
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
pub struct Proposal<Tx>
where
    Tx: AuthenticatedMantleTx + Clone,
{
    pub header: Header,
    pub transactions: Transactions<Tx>,
    pub signature: ed25519_dalek::Signature,
}

#[derive(Clone, Debug)]
pub struct Transactions<Tx>
where
    Tx: AuthenticatedMantleTx + Clone,
{
    pub service_reward: Option<Tx>,
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

#[derive(Debug)]
pub struct Builder<Tx> {
    header: Option<Header>,
    transactions: Vec<Tx>,
    service_reward: Option<Tx>,
}

pub struct BuilderWithSigningKey<Tx> {
    header: Header,
    transactions: Vec<Tx>,
    service_reward: Option<Tx>,
    signing_key: ed25519_dalek::SigningKey,
}

#[derive(Debug)]
pub struct BuilderWithSignature<Tx> {
    header: Header,
    transactions: Vec<Tx>,
    service_reward: Option<Tx>,
    signature: ed25519_dalek::Signature,
}

impl<Tx> Builder<Tx> {
    const fn new() -> Self {
        Self {
            header: None,
            transactions: Vec::new(),
            service_reward: None,
        }
    }

    pub fn header(mut self, header: Header) -> Result<Self, Error> {
        if !header.is_valid_bedrock_version() {
            return Err(Error::Signing("Invalid header version".to_owned()));
        }

        self.header = Some(header);

        Ok(self)
    }

    pub fn transactions(mut self, transactions: Vec<Tx>) -> Result<Self, Error> {
        if transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::Signing(format!(
                "Too many transactions (max {MAX_TRANSACTIONS})"
            )));
        }

        self.transactions = transactions;

        Ok(self)
    }

    #[must_use]
    pub fn service_reward(mut self, service_reward: Option<Tx>) -> Self {
        self.service_reward = service_reward;
        self
    }

    pub fn signing_key(
        self,
        signing_key: ed25519_dalek::SigningKey,
    ) -> Result<BuilderWithSigningKey<Tx>, Error> {
        let header = self
            .header
            .ok_or_else(|| Error::Signing("Header must be set before signing key".to_owned()))?;

        Ok(BuilderWithSigningKey {
            header,
            transactions: self.transactions,
            service_reward: self.service_reward,
            signing_key,
        })
    }

    pub fn signature(
        self,
        signature: ed25519_dalek::Signature,
    ) -> Result<BuilderWithSignature<Tx>, Error> {
        let header = self
            .header
            .ok_or_else(|| Error::Signing("Header must be set before signature".to_owned()))?;

        Ok(BuilderWithSignature {
            header,
            transactions: self.transactions,
            service_reward: self.service_reward,
            signature,
        })
    }
}

impl<Tx> BuilderWithSigningKey<Tx> {
    pub fn build(self) -> Result<Block<Tx>, Error>
    where
        Tx: Transaction,
        Tx::Hash: Into<Fr>,
    {
        let signature = self.header.sign(&self.signing_key)?;

        let leader_public_key = self.header.leader_proof().leader_key();
        let header_bytes = <Header as SerdeOp>::serialize(&self.header)?;

        leader_public_key
            .verify(&header_bytes, &signature)
            .map_err(|e| Error::Signing(format!("Signature verification failed: {e}")))?;

        Ok(Block::new_unchecked(
            self.header,
            self.transactions,
            self.service_reward,
            signature,
        ))
    }
}

impl<Tx> BuilderWithSignature<Tx> {
    pub fn build(self) -> Result<Block<Tx>, Error>
    where
        Tx: Transaction,
        Tx::Hash: Into<Fr>,
    {
        let leader_public_key = self.header.leader_proof().leader_key();
        let header_bytes = <Header as SerdeOp>::serialize(&self.header)?;
        leader_public_key
            .verify(&header_bytes, &self.signature)
            .map_err(|e| Error::Signing(format!("Signature verification failed: {e}")))?;

        Ok(Block::new_unchecked(
            self.header,
            self.transactions,
            self.service_reward,
            self.signature,
        ))
    }
}

impl<Tx> Proposal<Tx>
where
    Tx: AuthenticatedMantleTx + Clone,
{
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    #[must_use]
    pub const fn transactions(&self) -> &Transactions<Tx> {
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
    /// Private constructor - use `Builder` to create valid blocks
    const fn new_unchecked(
        header: Header,
        transactions: Vec<Tx>,
        service_reward: Option<Tx>,
        signature: ed25519_dalek::Signature,
    ) -> Self {
        Self {
            header,
            signature,
            service_reward,
            transactions,
        }
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

    pub fn validate(&self) -> Result<(), Error>
    where
        Tx: Transaction,
        Tx::Hash: Into<TxHash>,
    {
        if !self.header.is_valid_bedrock_version() {
            return Err(Error::Signing("Invalid header version".to_owned()));
        }

        if self.transactions.len() > MAX_TRANSACTIONS {
            return Err(Error::Signing(format!(
                "Too many transactions (max {MAX_TRANSACTIONS})"
            )));
        }

        let leader_public_key = self.header.leader_proof().leader_key();
        let header_bytes = <Header as SerdeOp>::serialize(&self.header)?;

        leader_public_key
            .verify(&header_bytes, &self.signature)
            .map_err(|e| Error::Signing(format!("Invalid signature: {e}")))?;

        Ok(())
    }

    #[must_use]
    pub const fn builder() -> Builder<Tx> {
        Builder::new()
    }

    pub fn to_proposal(&self) -> Proposal<Tx>
    where
        Tx: AuthenticatedMantleTx + Clone,
    {
        let mempool_transactions: Vec<TxHash> =
            self.transactions.iter().map(Transaction::hash).collect();

        let transactions = Transactions {
            service_reward: self.service_reward.clone(),
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

    use cryptarchia_engine::Slot;
    use ed25519_dalek::SigningKey;
    use num_bigint::BigUint;

    use super::*;
    use crate::{
        header::ContentId,
        mantle::{
            ledger::{Note, Tx, Utxo},
            ops::leader_claim::VoucherCm,
        },
        proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
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

    fn create_header() -> Header {
        Header::new(
            [0u8; 32].into(),
            ContentId::from([1u8; 32]),
            Slot::from(42u64),
            create_proof(),
        )
    }

    fn create_signature(header: &Header) -> ed25519_dalek::Signature {
        let signing_key = SigningKey::from_bytes(&[0; 32]);
        header.sign(&signing_key).expect("Signing should work")
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
        let header = create_header();
        let transactions: Vec<Tx> = vec![];
        let service_reward = None;

        let valid_signature = create_signature(&header);
        let _valid_block = Block::builder()
            .header(header.clone())
            .and_then(|b| b.transactions(transactions.clone()))
            .map(|b| b.service_reward(service_reward.clone()))
            .and_then(|b| b.signature(valid_signature))
            .and_then(BuilderWithSignature::build)
            .expect("Valid block should be created");

        // Block is guaranteed to be valid since it was created through the builder

        let wrong_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let invalid_signature = header
            .sign(&wrong_signing_key)
            .expect("Signing should work");

        let invalid_block_result = Block::builder()
            .header(header)
            .and_then(|b| b.transactions(transactions))
            .map(|b| b.service_reward(service_reward))
            .and_then(|b| b.signature(invalid_signature))
            .and_then(BuilderWithSignature::build);

        assert!(
            invalid_block_result.is_err(),
            "Should not create valid block with invalid signature"
        );
    }

    #[test]
    fn test_block_transaction_count_validation() {
        let header = create_header();
        let signature = create_signature(&header);
        let service_reward = None;

        let _valid_block: Block<Tx> = Block::builder()
            .header(header.clone())
            .and_then(|b| b.transactions(vec![]))
            .map(|b| b.service_reward(service_reward))
            .and_then(|b| b.signature(signature))
            .and_then(BuilderWithSignature::build)
            .expect("Valid block should be created");

        // Block is guaranteed to be valid since it was created through the builder

        let invalid_block_result = Block::builder()
            .header(header)
            .and_then(|b| b.transactions(create_transactions(MAX_TRANSACTIONS + 1)));

        assert!(invalid_block_result.is_err());
        let error_msg = invalid_block_result.unwrap_err().to_string();

        assert!(error_msg.contains("Too many transactions"));
        assert!(error_msg.contains(&MAX_TRANSACTIONS.to_string()));
    }
}
