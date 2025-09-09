use ::serde::{de::DeserializeOwned, Deserialize, Serialize};
use bytes::Bytes;
use ed25519_dalek::{ed25519::signature::Signer as _, SigningKey};
use groth16::Fr;

use crate::{header::Header, mantle::Transaction};

mod wire;

pub type TxHash = [u8; 32];
pub type BlockNumber = u64;
pub type SessionNumber = u64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to serialize: {0}")]
    Serialisation(#[from] crate::wire::Error),
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
    pub mempool_transactions: Vec<Fr>, // 1024 - len(service_reward)
}

/// A block
#[derive(Clone, Debug)]
pub struct Block<Tx> {
    header: Header,
    transactions: Vec<Tx>,
    pub service_reward: Option<Fr>,
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
    pub const fn new(header: Header, transactions: Vec<Tx>) -> Self {
        Self {
            header,
            transactions,
            service_reward: None,
        }
    }

    #[must_use]
    pub const fn new_with_service_reward(
        header: Header,
        transactions: Vec<Tx>,
        service_reward: Fr,
    ) -> Self {
        Self {
            header,
            transactions,
            service_reward: Some(service_reward),
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

    pub fn to_proposal(&self, signing_key: &SigningKey) -> Result<Proposal, Error>
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

        let header_bytes = crate::wire::serialize(&self.header)?;
        let signature = signing_key.sign(&header_bytes);

        Ok(Proposal {
            header: self.header.clone(),
            references,
            signature,
        })
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Bytes> for Block<Tx> {
    type Error = crate::wire::Error;

    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        crate::wire::deserialize(&bytes)
    }
}

impl<Tx: Clone + Eq + Serialize + DeserializeOwned> TryFrom<Block<Tx>> for Bytes {
    type Error = crate::wire::Error;

    fn try_from(block: Block<Tx>) -> Result<Self, Self::Error> {
        let serialized = crate::wire::serialize(&block)?;
        Ok(serialized.into())
    }
}
