use std::{
    collections::{BTreeMap, HashMap, HashSet},
    marker::PhantomData,
    num::NonZeroUsize,
    ops::RangeInclusive,
};

use async_trait::async_trait;
use bytes::Bytes;
use cryptarchia_engine::Slot;
use libp2p_identity::PeerId;
use nomos_core::{block::BlockNumber, header::HeaderId};
use thiserror::Error;

use super::{StorageBackend, StorageSerde, StorageTransaction};
use crate::api::{chain::StorageChainApi, da::StorageDaApi, StorageBackendApi};

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

    fn new(_config: Self::Settings) -> Result<Self, <Self as StorageBackend>::Error> {
        Ok(Self {
            inner: HashMap::new(),
            _serde_op: PhantomData,
        })
    }

    async fn store(
        &mut self,
        key: Bytes,
        value: Bytes,
    ) -> Result<(), <Self as StorageBackend>::Error> {
        let _ = self.inner.insert(key, value);
        Ok(())
    }

    async fn load(&mut self, key: &[u8]) -> Result<Option<Bytes>, <Self as StorageBackend>::Error> {
        Ok(self.inner.get(key).cloned())
    }

    async fn load_prefix(
        &mut self,
        _key: &[u8],
        _start_key: Option<&[u8]>,
        _end_key: Option<&[u8]>,
        _limit: Option<NonZeroUsize>,
    ) -> Result<Vec<Bytes>, <Self as StorageBackend>::Error> {
        unimplemented!()
    }

    async fn remove(
        &mut self,
        key: &[u8],
    ) -> Result<Option<Bytes>, <Self as StorageBackend>::Error> {
        Ok(self.inner.remove(key))
    }

    async fn execute(
        &mut self,
        transaction: Self::Transaction,
    ) -> Result<(), <Self as StorageBackend>::Error> {
        transaction(&mut self.inner);
        Ok(())
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for MockStorage<SerdeOp> {
    type Error = MockStorageError;
    type Block = Bytes;

    async fn get_block(
        &mut self,
        _header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error> {
        unimplemented!()
    }

    async fn store_block(
        &mut self,
        _header_id: HeaderId,
        _block: Self::Block,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn remove_block(
        &mut self,
        _header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error> {
        unimplemented!()
    }

    async fn store_immutable_block_ids(
        &mut self,
        _ids: BTreeMap<Slot, HeaderId>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_immutable_block_id(
        &mut self,
        _slot: Slot,
    ) -> Result<Option<HeaderId>, Self::Error> {
        unimplemented!()
    }

    async fn scan_immutable_block_ids(
        &mut self,
        _slot_range: RangeInclusive<Slot>,
        _limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, Self::Error> {
        unimplemented!()
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageDaApi for MockStorage<SerdeOp> {
    type Error = MockStorageError;
    type BlobId = [u8; 32];
    type Share = Bytes;
    type Commitments = Bytes;
    type ShareIndex = [u8; 2];
    type Id = PeerId;
    type NetworkId = u16;

    async fn get_light_share(
        &mut self,
        _blob_id: Self::BlobId,
        _share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, Self::Error> {
        unimplemented!()
    }

    async fn get_blob_share_indices(
        &mut self,
        _blob_id: Self::BlobId,
    ) -> Result<Option<HashSet<Self::ShareIndex>>, Self::Error> {
        unimplemented!()
    }

    async fn store_light_share(
        &mut self,
        _blob_id: Self::BlobId,
        _share_idx: Self::ShareIndex,
        _light_share: Self::Share,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_shared_commitments(
        &mut self,
        _blob_id: Self::BlobId,
    ) -> Result<Option<Self::Commitments>, Self::Error> {
        unimplemented!()
    }

    async fn store_shared_commitments(
        &mut self,
        _blob_id: Self::BlobId,
        _shared_commitments: Self::Commitments,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_blob_light_shares(
        &mut self,
        _blob_id: Self::BlobId,
    ) -> Result<Option<Vec<Self::Share>>, Self::Error> {
        unimplemented!()
    }

    async fn store_assignations(
        &mut self,
        _block_number: BlockNumber,
        _assignations: HashMap<Self::NetworkId, HashSet<Self::Id>>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_assignations(
        &mut self,
        _block_number: BlockNumber,
    ) -> Result<HashMap<Self::NetworkId, HashSet<Self::Id>>, Self::Error> {
        unimplemented!()
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for MockStorage<SerdeOp> {}
