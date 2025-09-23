use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use bytes::Bytes;
use ed25519_dalek::Verifier as _;
use groth16::Fr;

use crate::{
    codec::SerdeOp, header::Header, mantle::Transaction, proofs::leader_proof::LeaderProof as _,
};

const MAX_TRANSACTIONS: usize = 1024;

mod wire;

pub type TxHash = [u8; 32];
pub type BlockNumber = u64;
pub type SessionNumber = u64;

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to serialize: {0}")]
    Serialisation(#[from] crate::codec::Error),
    #[error("Signing error: {0}")]
    Signing(String),
}

/// A block proposal
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub header: Header,
    pub references: References,
    pub signature: ed25519_dalek::Signature,
}

#[derive(Clone, Debug)]
pub struct References {
    pub service_reward: Option<Fr>,
    pub mempool_transactions: Vec<Fr>, // MAX_TRANSACTIONS - len(service_reward)
}

/// A block
#[derive(Clone, Debug)]
pub struct Block<Tx> {
    header: Header,
    signature: ed25519_dalek::Signature,
    service_reward: Option<Fr>,
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
    pub const fn signature(&self) -> &ed25519_dalek::Signature {
        &self.signature
    }
}

impl<Tx> Block<Tx> {
    #[must_use]
    pub const fn new(
        header: Header,
        transactions: Vec<Tx>,
        service_reward: Option<Fr>,
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
    pub const fn service_reward(&self) -> Option<Fr> {
        self.service_reward
    }

    #[must_use]
    pub const fn signature(&self) -> &ed25519_dalek::Signature {
        &self.signature
    }

    /// Basic validation
    pub fn validate(&self) -> Result<(), Error>
    where
        Tx: Transaction,
        Tx::Hash: Into<Fr>,
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

    pub fn to_proposal(&self) -> Result<Proposal, Error>
    where
        Tx: Transaction,
        Tx::Hash: Into<Fr>,
    {
        let mempool_transactions: Vec<Fr> = self
            .transactions
            .iter()
            .map(|tx| tx.hash().into())
            .collect();

        let references = References {
            service_reward: self.service_reward,
            mempool_transactions,
        };

        Ok(Proposal {
            header: self.header.clone(),
            references,
            signature: self.signature,
        })
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
        let service_reward = Some(Fr::from(123u64));

        let valid_signature = create_signature(&header);
        let valid_block = Block::new(
            header.clone(),
            transactions.clone(),
            service_reward,
            valid_signature,
        );

        assert!(valid_block.validate().is_ok());

        let wrong_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let invalid_signature = header
            .sign(&wrong_signing_key)
            .expect("Signing should work");

        let invalid_block = Block::new(header, transactions, service_reward, invalid_signature);
        assert!(invalid_block.validate().is_err());
    }

    #[test]
    fn test_block_transaction_count_validation() {
        let header = create_header();
        let signature = create_signature(&header);
        let service_reward = Some(Fr::from(123u64));

        let valid_block: Block<Tx> = Block::new(header.clone(), vec![], service_reward, signature);
        assert!(valid_block.validate().is_ok());

        let invalid_block: Block<Tx> = Block::new(
            header,
            create_transactions(MAX_TRANSACTIONS + 1),
            service_reward,
            signature,
        );
        let result = invalid_block.validate();

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Too many transactions"));
        assert!(error_msg.contains(&MAX_TRANSACTIONS.to_string()));
    }
}
