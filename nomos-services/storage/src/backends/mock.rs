use std::{
    collections::{BTreeMap, HashMap, HashSet},
    num::NonZeroUsize,
    ops::RangeInclusive,
};

use async_trait::async_trait;
use bytes::Bytes;
use cryptarchia_engine::Slot;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use nomos_core::{block::SessionNumber, header::HeaderId, mantle::TxHash};
use thiserror::Error;

use super::{StorageBackend, StorageTransaction};
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
pub struct MockStorage {
    inner: HashMap<Bytes, Bytes>,
}

impl core::fmt::Debug for MockStorage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        format!("MockStorage {{ inner: {:?} }}", self.inner).fmt(f)
    }
}

#[async_trait]
impl StorageBackend for MockStorage {
    type Settings = ();
    type Error = MockStorageError;
    type Transaction = MockStorageTransaction;

    fn new(_config: Self::Settings) -> Result<Self, <Self as StorageBackend>::Error> {
        Ok(Self {
            inner: HashMap::new(),
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

    async fn bulk_store(
        &mut self,
        items: HashMap<Bytes, Bytes>,
    ) -> Result<(), <Self as StorageBackend>::Error> {
        for (key, value) in items {
            let _ = self.inner.insert(key, value);
        }
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
impl StorageChainApi for MockStorage {
    type Error = MockStorageError;
    type Block = Bytes;
    type Tx = ();

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

    async fn store_transactions(
        &mut self,
        _transactions: HashMap<TxHash, Self::Tx>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_transactions(
        &mut self,
        _tx_hashes: &[TxHash],
    ) -> Result<Vec<Self::Tx>, Self::Error> {
        unimplemented!()
    }

    async fn remove_transactions(&mut self, _tx_hashes: &[TxHash]) -> Result<(), Self::Error> {
        unimplemented!()
    }
}

#[async_trait]
impl StorageDaApi for MockStorage {
    type Error = MockStorageError;
    type BlobId = [u8; 32];
    type Share = Bytes;
    type Commitments = Bytes;
    type Tx = ();
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
        _session_id: SessionNumber,
        _assignations: HashMap<Self::NetworkId, HashSet<Self::Id>>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_assignations(
        &mut self,
        _session_id: SessionNumber,
    ) -> Result<Option<HashMap<Self::NetworkId, HashSet<Self::Id>>>, Self::Error> {
        unimplemented!()
    }

    async fn store_addresses(
        &mut self,
        _ids: HashMap<Self::Id, Multiaddr>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn get_address(&mut self, _id: Self::Id) -> Result<Option<Multiaddr>, Self::Error> {
        unimplemented!()
    }

    async fn get_tx(
        &mut self,
        _blob_id: Self::BlobId,
    ) -> Result<Option<(u16, Self::Tx)>, Self::Error> {
        unimplemented!()
    }

    async fn store_tx(
        &mut self,
        _blob_id: Self::BlobId,
        _assignations: u16,
        _tx: Self::Tx,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }
}

#[async_trait]
impl StorageBackendApi for MockStorage {}
