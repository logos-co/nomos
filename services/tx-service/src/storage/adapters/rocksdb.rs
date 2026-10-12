use std::{marker::PhantomData, pin::Pin};

use async_trait::async_trait;
use futures::Stream;
use lb_core::{
    header::HeaderId,
    mantle::{
        traits::{Hashable, StorageSize},
        transactions::hash::TxHash,
    },
};
use lb_cryptarchia_engine::Slot;
use lb_storage_service::{StorageService, api::StorageApi};
use overwatch::services::{ServiceData, relay::OutboundRelay};
use serde::{Deserialize, Serialize};

use crate::{backend::MempoolError, storage::MempoolStorageAdapter};

/// Maps the mempool's item/key interface to the shared storage API.
#[derive(Clone)]
pub struct RocksStorageAdapter<Item, Key> {
    storage: StorageApi<Item>,
    key: PhantomData<fn() -> Key>,
}

#[async_trait]
impl<Item, Key, RuntimeServiceId> MempoolStorageAdapter<RuntimeServiceId>
    for RocksStorageAdapter<Item, Key>
where
    Item: Clone
        + Eq
        + Send
        + Sync
        + 'static
        + Serialize
        + for<'de> Deserialize<'de>
        + Hashable<Hash = TxHash>
        + StorageSize,
    Key: Clone + Send + Sync + 'static + Into<TxHash>,
{
    type Item = Item;
    type Key = Key;
    type BlockId = HeaderId;
    type Error = MempoolError;

    fn new(
        storage_relay: OutboundRelay<<StorageService<RuntimeServiceId> as ServiceData>::Message>,
    ) -> Self {
        Self {
            storage: StorageApi::new(storage_relay),
            key: PhantomData,
        }
    }

    async fn store_item(&mut self, key: Self::Key, item: Self::Item) -> Result<(), Self::Error> {
        self.storage
            .store_transaction(key.into(), item)
            .await
            .map_err(MempoolError::DynamicPoolError)
    }

    async fn get_items(
        &self,
        keys: &[Self::Key],
    ) -> Result<Pin<Box<dyn Stream<Item = Self::Item> + Send>>, Self::Error> {
        if keys.is_empty() {
            return Ok(Box::pin(futures::stream::empty()));
        }
        let hashes = keys.iter().cloned().map(Into::into).collect();
        let items = self
            .storage
            .get_transactions(hashes)
            .await
            .map_err(MempoolError::DynamicPoolError)?;
        Ok(items)
    }

    async fn remove_items(&mut self, keys: &[Self::Key]) -> Result<(), Self::Error> {
        let hashes: Vec<TxHash> = keys.iter().cloned().map(Into::into).collect();
        self.storage
            .remove_transactions(&hashes)
            .await
            .map_err(MempoolError::DynamicPoolError)
    }

    async fn get_block_slot(&self, block_id: &Self::BlockId) -> Result<Option<Slot>, Self::Error> {
        let block = self
            .storage
            .try_get_block(block_id)
            .await
            .map_err(MempoolError::DynamicPoolError)?;
        Ok(block.map(|block| block.header().slot()))
    }
}
