use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use lb_cryptarchia_engine::Slot;

pub mod adapters;

#[async_trait]
pub trait MempoolStorageAdapter<RuntimeServiceId>: Send + Sync {
    type Item: Send;

    type Key: Send + Sync;

    type BlockId: Send + Sync;

    type Error: Send;

    fn new(
        storage_relay: overwatch::services::relay::OutboundRelay<
            <lb_storage_service::StorageService<RuntimeServiceId> as overwatch::services::ServiceData>::Message,
        >,
    ) -> Self;

    async fn store_item(&mut self, key: Self::Key, item: Self::Item) -> Result<(), Self::Error>;

    async fn get_items(
        &self,
        keys: &[Self::Key],
    ) -> Result<Pin<Box<dyn Stream<Item = Self::Item> + Send>>, Self::Error>;

    async fn remove_items(&mut self, keys: &[Self::Key]) -> Result<(), Self::Error>;

    /// Slot of the stored block `block_id`, or `None` if it is not stored.
    async fn get_block_slot(&self, block_id: &Self::BlockId) -> Result<Option<Slot>, Self::Error>;
}
