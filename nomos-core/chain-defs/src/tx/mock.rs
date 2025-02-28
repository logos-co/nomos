use std::convert::Infallible;

use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use bytes::{Bytes, BytesMut};
use serde::Serialize;

use crate::{
    tx::{Transaction, TransactionHasher},
    wire,
    wire::serialize,
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MockTransaction<M> {
    id: MockTxId,
    content: M,
}

impl<M: Serialize> MockTransaction<M> {
    pub fn new(content: M) -> Self {
        let id = MockTxId::try_from(serialize(&content).unwrap().as_slice()).unwrap();
        Self { id, content }
    }

    pub const fn message(&self) -> &M {
        &self.content
    }

    pub const fn id(&self) -> MockTxId {
        self.id
    }

    fn as_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        wire::serializer_into_buffer(&mut buff)
            .serialize_into(&self)
            .expect("MockTransaction serialization to buffer failed");
        buff.freeze()
    }
}

impl<M: Serialize> Transaction for MockTransaction<M> {
    const HASHER: TransactionHasher<Self> = Self::id;
    type Hash = MockTxId;

    fn as_bytes(&self) -> Bytes {
        Self::as_bytes(self)
    }
}

#[expect(clippy::fallible_impl_from)]
impl<M: Serialize> From<M> for MockTransaction<M> {
    fn from(msg: M) -> Self {
        let id = MockTxId::try_from(serialize(&msg).unwrap().as_slice()).unwrap();
        Self { id, content: msg }
    }
}

#[derive(
    Debug, Eq, Hash, PartialEq, Ord, Copy, Clone, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct MockTxId([u8; 32]);

impl From<[u8; 32]> for MockTxId {
    fn from(tx_id: [u8; 32]) -> Self {
        Self(tx_id)
    }
}

impl core::ops::Deref for MockTxId {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for MockTxId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl MockTxId {
    pub const fn new(tx_id: [u8; 32]) -> Self {
        Self(tx_id)
    }
}

impl TryFrom<&[u8]> for MockTxId {
    type Error = Infallible;

    fn try_from(msg: &[u8]) -> Result<Self, Self::Error> {
        let mut hasher = Blake2bVar::new(32).unwrap();
        hasher.update(msg);
        let mut id = [0u8; 32];
        hasher.finalize_variable(&mut id).unwrap();
        Ok(Self(id))
    }
}

impl<M> From<&MockTransaction<M>> for MockTxId {
    fn from(msg: &MockTransaction<M>) -> Self {
        msg.id
    }
}
