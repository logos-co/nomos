pub mod adapters;

use std::collections::BTreeMap;

use cryptarchia_engine::Slot;
use futures::future::join_all;
use nomos_core::header::HeaderId;
use nomos_storage::{backends::StorageBackend, StorageService};
use overwatch::services::{relay::OutboundRelay, ServiceData};

#[async_trait::async_trait]
pub trait StorageAdapter<RuntimeServiceId> {
    type Backend: StorageBackend + Send + Sync + 'static;
    type Block: Send;

    async fn new(
        network_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    /// Sends a store message to the storage service to retrieve a block by its
    /// header id
    ///
    /// # Returns
    ///
    /// The block with the given header id. If no block is found, returns None.
    async fn get_block(&self, key: &HeaderId) -> Option<Self::Block>;

    async fn store_block(
        &self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), overwatch::DynError>;

    /// Remove a block from the storage layer.
    ///
    /// * If the block exists, this function returns `Ok(Self::Block).`
    /// * If the block does not exist, this function returns `Ok(None)`.
    /// * If an error occurs, this function returns `Err(overwatch::DynError)`.
    async fn remove_block(
        &self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, overwatch::DynError>;

    /// Remove a batch of blocks from the storage layer.
    ///
    /// For each block being deleted:
    /// * If the block exists, this function returns `Ok(Self::Block).`
    /// * If the block does not exist, this function returns `Ok(None)`.
    /// * If an error occurs, this function returns `Err(overwatch::DynError)`.
    async fn remove_blocks<Headers>(
        &self,
        header_ids: Headers,
    ) -> impl Iterator<Item = Result<Option<Self::Block>, overwatch::DynError>>
    where
        Headers: Iterator<Item = HeaderId> + Send,
    {
        join_all(header_ids.map(|header_id| async move { self.remove_block(header_id).await }))
            .await
            .into_iter()
    }

    /// Store immutable block ids with their slots.
    async fn store_immutable_block_ids(
        &self,
        blocks: BTreeMap<Slot, HeaderId>,
    ) -> Result<(), overwatch::DynError>;
}
