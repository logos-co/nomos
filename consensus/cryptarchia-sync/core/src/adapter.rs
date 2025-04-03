use cryptarchia_engine::Slot;
use futures::Stream;
use nomos_core::block::AbstractBlock;

#[async_trait::async_trait]
pub trait CryptarchiaAdapter {
    type Block: AbstractBlock;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaAdapterError>;

    fn tip_slot(&self) -> Slot;

    fn has_block(&self, id: &<Self::Block as AbstractBlock>::Id) -> bool;
}

#[derive(thiserror::Error, Debug)]
pub enum CryptarchiaAdapterError {
    #[error("Parent not found")]
    ParentNotFound,
    #[error("Invalid block: {0}")]
    InvalidBlock(Box<dyn std::error::Error + Send + Sync>),
}

type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter {
    type Block: AbstractBlock;

    async fn fetch_blocks_from_slot(
        &self,
        start_slot: Slot,
    ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>>;

    async fn fetch_chain_backward(
        &self,
        tip: <Self::Block as AbstractBlock>::Id,
    ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>>;
}
