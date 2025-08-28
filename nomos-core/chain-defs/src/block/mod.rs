pub mod builder;

use ::serde::{de::DeserializeOwned, Deserialize, Serialize};
use bytes::Bytes;

use crate::{header::Header, wire};

pub type TxHash = [u8; 32];

pub type BlockNumber = u64;

pub type SessionNumber = u64;

/// A block
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block<Tx: Clone + Eq, BlobCertificate: Clone + Eq> {
    header: Header,
    cl_transactions: Vec<Tx>,
    bl_blobs: Vec<BlobCertificate>,
}

impl<Tx: Clone + Eq, BlobCertificate: Clone + Eq> Block<Tx, BlobCertificate> {
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    pub fn transactions(&self) -> impl Iterator<Item = &Tx> + '_ {
        self.cl_transactions.iter()
    }

    pub fn blobs(&self) -> impl Iterator<Item = &BlobCertificate> + '_ {
        self.bl_blobs.iter()
    }
}

impl<
        Tx: Clone + Eq + Serialize + DeserializeOwned,
        BlobCertificate: Clone + Eq + Serialize + DeserializeOwned,
    > Block<Tx, BlobCertificate>
{
    /// Encode block into bytes
    #[must_use]
    pub fn as_bytes(&self) -> Bytes {
        wire::serialize(self).unwrap().into()
    }

    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        wire::deserialize(bytes).unwrap()
    }

    #[must_use]
    pub const fn cl_transactions_len(&self) -> usize {
        self.cl_transactions.len()
    }

    #[must_use]
    pub const fn bl_blobs_len(&self) -> usize {
        self.bl_blobs.len()
    }
}

impl<
        Tx: Clone + Eq + Serialize + DeserializeOwned,
        BlobCertificate: Clone + Eq + Serialize + DeserializeOwned,
    > TryFrom<Bytes> for Block<Tx, BlobCertificate>
{
    type Error = wire::Error;

    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        wire::deserialize(&bytes)
    }
}

impl<
        Tx: Clone + Eq + Serialize + DeserializeOwned,
        BlobCertificate: Clone + Eq + Serialize + DeserializeOwned,
    > TryFrom<Block<Tx, BlobCertificate>> for Bytes
{
    type Error = wire::Error;

    fn try_from(block: Block<Tx, BlobCertificate>) -> Result<Self, Self::Error> {
        let serialized = wire::serialize(&block)?;
        Ok(serialized.into())
    }
}
