use cryptarchia_consensus::ConsensusMsg;
use futures::Stream;
use nomos_core::block::Block;
use overwatch::services::relay::OutboundRelay;
use tokio::sync::oneshot;
use tokio_stream::{wrappers::BroadcastStream, StreamExt as _};
use tracing::debug;

use crate::consensus::ConsensusAdapter;

pub struct CryptarchiaConsensusAdapter<Tx>
where
    Tx: Clone + Eq + std::hash::Hash,
{
    consensus_relay: OutboundRelay<ConsensusMsg<Block<Tx>>>,
}

#[async_trait::async_trait]
impl<Tx> ConsensusAdapter for CryptarchiaConsensusAdapter<Tx>
where
    Tx: Clone + Eq + std::hash::Hash + Send + Sync + 'static + std::fmt::Debug,
{
    type Tx = Tx;

    async fn new(consensus_relay: OutboundRelay<ConsensusMsg<Block<Self::Tx>>>) -> Self {
        Self { consensus_relay }
    }

    async fn block_stream(&self) -> Box<dyn Stream<Item = Block<Self::Tx>> + Unpin + Send> {
        let (sender, receiver) = oneshot::channel();

        self.consensus_relay
            .send(ConsensusMsg::BlockSubscribe { sender })
            .await
            .expect("Failed to send BlockSubscribe message");

        let broadcast_receiver = receiver
            .await
            .expect("Failed to receive broadcast receiver");

        Box::new(
            BroadcastStream::new(broadcast_receiver).filter_map(|result| match result {
                Ok(block) => Some(block),
                Err(e) => {
                    debug!("Unrecognized message: {e}");
                    None
                }
            }),
        )
    }
}
