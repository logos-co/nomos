use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use bytes::Bytes;
use poseidon2::Fr;

use crate::{
    codec::{DeserializeOp as _, SerializeOp as _},
    header::Header,
};
pub type TxHash = [u8; 32];
pub type BlockNumber = u64;

/// A block proposal
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block<Tx> {
    header: Header,
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
    pub const fn new(header: Header, transactions: Vec<Tx>) -> Self {
        Self {
            header,
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
