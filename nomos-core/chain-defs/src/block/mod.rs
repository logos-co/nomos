use ::serde::{de::DeserializeOwned, Deserialize, Serialize};
use bytes::Bytes;
use ed25519_dalek::{Signer as _, SigningKey};
use groth16::Fr;

use crate::{codec::SerdeOp, header::Header, mantle::Transaction};

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
    pub mempool_transactions: Vec<Fr>, // 1024 - len(service_reward)
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

        if self.transactions.len() > 1024 {
            return Err(Error::Signing(
                "Too many transactions (max 1024)".to_owned(),
            ));
        }

        // TODO: validate signature?

        Ok(())
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

        let header_bytes = crate::codec::bincode::serialize(&self.header)?;
        let signature = signing_key.sign(&header_bytes);

        Ok(Proposal {
            header: self.header.clone(),
            references,
            signature,
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
