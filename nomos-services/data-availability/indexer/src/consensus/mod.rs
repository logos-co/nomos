pub mod adapters;

use chain_service::ConsensusMsg;
use futures::Stream;
use nomos_core::block::Block;
use overwatch::services::relay::OutboundRelay;

#[async_trait::async_trait]
pub trait ConsensusAdapter {
    type Tx: Clone + Eq;
    type Cert: Clone + Eq;

    async fn new(consensus_relay: OutboundRelay<ConsensusMsg<Block<Self::Tx, Self::Cert>>>)
        -> Self;

    async fn block_stream(
        &self,
    ) -> Box<dyn Stream<Item = Block<Self::Tx, Self::Cert>> + Unpin + Send>;
}
