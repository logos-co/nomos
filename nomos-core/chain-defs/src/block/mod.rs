pub mod builder;

use ::serde::{de::DeserializeOwned, Deserialize, Serialize};
use bytes::Bytes;
use core::hash::Hash;
use cryptarchia_engine::Slot;
use indexmap::IndexSet;
use std::fmt::Debug;

use crate::{
    header::{Header, HeaderId},
    wire,
};

// TODO: Rename this to Block
//       by renaming the existing Block to something else.
pub trait AbstractBlock {
    type Id: Eq + Hash + Copy + Debug;

    fn id(&self) -> Self::Id;
    fn parent(&self) -> Self::Id;
    fn slot(&self) -> Slot;
}

pub type TxHash = [u8; 32];

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
    type Id = HeaderId;

    fn id(&self) -> Self::Id {
        self.header().id()
    }

    fn parent(&self) -> Self::Id {
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

// TODO: We need to implement Hash and Eq for `Block` if using `Block` in type definitions instead of Tx and BlobCertificate
// Seems related to Overwatch and RuntimeServiceId somehow.
impl<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> PartialEq
    for Block<Tx, BlobCertificate>
{
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> Eq for Block<Tx, BlobCertificate> {}

//TODO: Implementing Hash for Block
impl<Tx: Clone + Eq + Hash, BlobCertificate: Clone + Eq + Hash> Hash
    for Block<Tx, BlobCertificate>
{
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {
        todo!()
    }
}
