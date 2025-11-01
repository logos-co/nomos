pub mod sampling;

use std::pin::Pin;

use futures::Stream;
use nomos_core::da::BlobId;
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use tx_service::backend::MempoolError;

#[derive(thiserror::Error, Debug)]
pub enum MempoolAdapterError {
    #[error("Mempool responded with and error: {0}")]
    Mempool(#[from] MempoolError),
    #[error("Channel receive error: {0}")]
    ChannelRecv(#[from] tokio::sync::oneshot::error::RecvError),
    #[error("Other mempool adapter error: {0}")]
    Other(DynError),
}

#[async_trait::async_trait]
pub trait DaMempoolAdapter {
    type MempoolService: ServiceData;
    type Tx;

    fn new(outbound_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self;

    async fn subscribe(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = BlobId> + Send>>, MempoolAdapterError>;
}
