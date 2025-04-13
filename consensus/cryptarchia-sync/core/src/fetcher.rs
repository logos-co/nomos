use cryptarchia_engine::Slot;
use futures::Stream;
use nomos_core::{block::AbstractBlock, header::HeaderId};

pub type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

/// Abstracts over the block fetching operations,
/// which can be implemented by any network protocol.
#[async_trait::async_trait]
pub trait BlockFetcher {
    type Block: AbstractBlock;
    type ProviderId: Clone + Send + Sync;

    /// Streams blocks starting from the given slot.
    async fn fetch_blocks_forward(
        &self,
        start_slot: Slot,
    ) -> Result<
        BoxedStream<(Self::Block, Self::ProviderId)>,
        Box<dyn std::error::Error + Send + Sync>,
    >;

    /// Streams a chain of blocks in reverse order from the given tip.
    async fn fetch_chain_backward(
        &self,
        tip: HeaderId,
        provider_id: Self::ProviderId,
    ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>>;
}
