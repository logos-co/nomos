pub mod requests;

use std::error::Error;

use async_trait::async_trait;
use nomos_core::header::HeaderId;

#[async_trait]
pub trait StorageChainApi {
    type Error: Error + Send + Sync + 'static;
    type Block: Send + Sync;

    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, Self::Error>;

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), Self::Error>;
}
