pub mod requests;

use async_trait::async_trait;
use overwatch::DynError;

#[async_trait]
pub trait StorageChainApi {
    type HeaderId;
    type Block;

    async fn get_block(
        &mut self,
        header_id: Self::HeaderId,
    ) -> Result<Option<Self::Block>, DynError>;

    async fn store_block(
        &mut self,
        header_id: Self::HeaderId,
        block: Self::Block,
    ) -> Result<(), DynError>;
}
