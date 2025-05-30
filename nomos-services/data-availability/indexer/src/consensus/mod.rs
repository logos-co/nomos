pub mod adapters;

use cryptarchia_consensus::ConsensusMsg;
use futures::Stream;
use nomos_core::block::Block;
use overwatch::services::relay::OutboundRelay;

#[async_trait::async_trait]
pub trait ConsensusAdapter {
    type Tx: Clone + Eq + std::hash::Hash;

    async fn new(consensus_relay: OutboundRelay<ConsensusMsg<Block<Self::Tx>>>) -> Self;

    async fn block_stream(&self) -> Box<dyn Stream<Item = Block<Self::Tx>> + Unpin + Send>;
}
