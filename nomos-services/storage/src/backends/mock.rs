use std::{collections::HashMap, marker::PhantomData};

use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;
use overwatch::DynError;
use thiserror::Error;

use super::{StorageBackend, StorageSerde, StorageTransaction};
use crate::api::{chain::StorageChainApi, da::StorageDaApi, StorageBackendApi, StorageFunctions};

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

    async fn remove(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        Ok(self.inner.remove(key))
    }

    async fn execute(&mut self, transaction: Self::Transaction) -> Result<(), Self::Error> {
        transaction(&mut self.inner);
        Ok(())
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for MockStorage<SerdeOp> {
    type Block = Bytes;

    async fn get_block(&mut self, _header_id: HeaderId) -> Result<Option<Self::Block>, DynError> {
        unimplemented!()
    }

    async fn store_block(
        &mut self,
        _header_id: HeaderId,
        _block: Self::Block,
    ) -> Result<(), DynError> {
        unimplemented!()
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageDaApi for MockStorage<SerdeOp> {
    type BlobId = [u8; 32];
    type Share = Bytes;
    type Commitments = Bytes;
    type ShareIndex = [u8; 2];

    async fn get_light_share(
        &mut self,
        _blob_id: Self::BlobId,
        _share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, DynError> {
        unimplemented!()
    }

    async fn store_light_share(
        &mut self,
        _blob_id: Self::BlobId,
        _share_idx: Self::ShareIndex,
        _light_share: Self::Share,
    ) -> Result<(), DynError> {
        unimplemented!()
    }

    async fn store_shared_commitments(
        &mut self,
        _blob_id: Self::BlobId,
        _shared_commitments: Self::Commitments,
    ) -> Result<(), DynError> {
        unimplemented!()
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageFunctions for MockStorage<SerdeOp> {}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for MockStorage<SerdeOp> {}
