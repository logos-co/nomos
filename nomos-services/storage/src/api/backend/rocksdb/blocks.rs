use std::{num::NonZeroUsize, ops::RangeInclusive};

use async_trait::async_trait;
use bytes::Bytes;
use cryptarchia_engine::Slot;
use nomos_core::header::HeaderId;

use crate::{
    api::{
        backend::rocksdb::{utils::key_bytes, Error},
        chain::StorageChainApi,
    },
    backends::{rocksdb::RocksBackend, StorageBackend as _, StorageSerde},
};

const IMMUTABLE_BLOCK_PREFIX: &str = "immutable_block/slot/";

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for RocksBackend<SerdeOp> {
    type Error = Error;
    type Block = Bytes;
    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.load(&key).await.map_err(Into::into)
    }

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.store(key, block).await.map_err(Into::into)
    }

    async fn remove_block(
        &mut self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error> {
        let encoded_header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&encoded_header_id);
        self.remove(&key).await.map_err(Into::into)
    }

    async fn store_immutable_block_id(
        &mut self,
        slot: Slot,
        header_id: HeaderId,
    ) -> Result<(), Self::Error> {
        let key = key_bytes(IMMUTABLE_BLOCK_PREFIX, slot.to_be_bytes());
        let header_id: [u8; 32] = header_id.into();
        self.store(key, Bytes::copy_from_slice(&header_id))
            .await
            .map_err(Into::into)
    }

    async fn get_immutable_block_id(
        &mut self,
        slot: Slot,
    ) -> Result<Option<HeaderId>, Self::Error> {
        let key = key_bytes(IMMUTABLE_BLOCK_PREFIX, slot.to_be_bytes());
        self.load(&key)
            .await?
            .map(|bytes| bytes.as_ref().try_into().map_err(Into::into))
            .transpose()
    }

    async fn scan_immutable_block_ids(
        &mut self,
        _slot_range: RangeInclusive<Slot>,
        _limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, Self::Error> {
        todo!("implement this by updating load_prefix to accept more arguments")
    }
}
