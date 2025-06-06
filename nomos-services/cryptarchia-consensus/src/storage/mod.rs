pub mod adapters;

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

    async fn remove_block(
        &self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, overwatch::DynError>;

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
}
