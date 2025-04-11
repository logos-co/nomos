pub mod builder;

use core::hash::Hash;

use ::serde::{de::DeserializeOwned, Deserialize, Serialize};
use bytes::Bytes;
use cryptarchia_engine::Slot;
use indexmap::IndexSet;

use crate::{
    header::{Header, HeaderId},
    wire,
};

pub type TxHash = [u8; 32];

/// A trait that defines interface for a block.
// TODO: Rename this to `Block` after getting agreement
pub trait AbstractBlock {
    fn id(&self) -> HeaderId;
    fn parent(&self) -> HeaderId;
    fn slot(&self) -> Slot;
}

/// A block
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> {
    header: Header,
    cl_transactions: IndexSet<Tx>,
    bl_blobs: IndexSet<BlobCertificate>,
}

impl<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> AbstractBlock
    for Block<Tx, BlobCertificate>
{
    fn id(&self) -> HeaderId {
        self.header().id()
    }

    fn parent(&self) -> HeaderId {
        self.header().parent()
    }

    fn slot(&self) -> Slot {
        self.header().slot()
    }
}

impl<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> Block<Tx, BlobCertificate> {
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
        Tx: Clone + Eq + Hash + Serialize + DeserializeOwned,
        BlobCertificate: Clone + Eq + Hash + Serialize + DeserializeOwned,
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
    pub fn cl_transactions_len(&self) -> usize {
        self.cl_transactions.len()
    }

    #[must_use]
    pub fn bl_blobs_len(&self) -> usize {
        self.bl_blobs.len()
    }
}
