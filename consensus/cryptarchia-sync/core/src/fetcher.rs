use cryptarchia_engine::Slot;
use futures::stream::BoxStream;
use nomos_core::{block::AbstractBlock, header::HeaderId};

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
    ) -> Result<BoxStream<(Self::Block, Self::ProviderId)>, Box<dyn std::error::Error + Send + Sync>>;

    /// Streams a chain of blocks in reverse order from the given tip.
    async fn fetch_chain_backward(
        &self,
        tip: HeaderId,
        provider_id: Self::ProviderId,
    ) -> Result<BoxStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>>;
}
