use std::{collections::HashMap, marker::PhantomData};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use thiserror::Error;

use super::{StorageBackend, StorageIterator, StorageSerde, StorageTransaction};

#[derive(Debug, Error)]
#[error("Errors in MockStorage should not happen")]
pub enum MockStorageError {}

pub type MockStorageTransaction = Box<dyn Fn(&mut HashMap<Bytes, Bytes>) + Send + Sync>;

impl StorageTransaction for MockStorageTransaction {
    type Result = ();
    type Transaction = Self;
}

//
pub struct MockStorage<SerdeOp> {
    inner: HashMap<Bytes, Bytes>,
    _serde_op: PhantomData<SerdeOp>,
}

impl<SerdeOp> core::fmt::Debug for MockStorage<SerdeOp> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        format!("MockStorage {{ inner: {:?} }}", self.inner).fmt(f)
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackend for MockStorage<SerdeOp> {
    type Settings = ();
    type Error = MockStorageError;
    type Transaction = MockStorageTransaction;
    type Iterator = MockIterator;
    type SerdeOperator = SerdeOp;

    fn new(_config: Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self {
            inner: HashMap::new(),
            _serde_op: PhantomData,
        })
    }

    async fn store(&mut self, key: Bytes, value: Bytes) -> Result<(), Self::Error> {
        let _ = self.inner.insert(key, value);
        Ok(())
    }

    async fn load(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        Ok(self.inner.get(key).cloned())
    }

    async fn load_prefix(&mut self, _key: &[u8]) -> Result<Vec<Bytes>, Self::Error> {
        unimplemented!()
    }

    async fn scan_range(&self, prefix: &[u8], start: &[u8]) -> Result<Self::Iterator, Self::Error> {
        unimplemented!()
    }

    async fn remove(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        Ok(self.inner.remove(key))
    }

    async fn execute(&mut self, transaction: Self::Transaction) -> Result<(), Self::Error> {
        transaction(&mut self.inner);
        Ok(())
    }
}

pub struct MockIterator;

impl StorageIterator for MockIterator {
    type Error = MockStorageError;

    fn stream(&self) -> BoxStream<'_, Result<(Bytes, Bytes), Self::Error>> {
        unimplemented!()
    }
}
