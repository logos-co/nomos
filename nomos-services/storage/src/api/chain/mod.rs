pub mod requests;

use async_trait::async_trait;
use nomos_core::header::HeaderId;
use overwatch::DynError;

#[async_trait]
pub trait StorageChainApi {
    type Block: Send + Sync;

    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, DynError>;

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), DynError>;
}
