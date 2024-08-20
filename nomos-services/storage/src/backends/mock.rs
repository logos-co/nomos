// std
use std::collections::HashMap;
use std::marker::PhantomData;
// crates
use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;
// internal
use super::{StorageBackend, StorageSerde, StorageTransaction};

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
    type SerdeOperator = SerdeOp;

    fn new(_config: Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self {
            inner: HashMap::new(),
            _serde_op: Default::default(),
        })
    }

    async fn store(&mut self, key: Bytes, value: Bytes) -> Result<(), Self::Error> {
        let _ = self.inner.insert(key, value);
        Ok(())
    }

    async fn load(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        Ok(self.inner.get(key).cloned())
    }

    async fn load_prefix(&mut self, _key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
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
