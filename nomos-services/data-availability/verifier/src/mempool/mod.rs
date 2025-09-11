pub mod kzgrs;

use nomos_mempool::backend::MempoolError;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

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

    async fn post_tx(&self, tx: Self::Tx) -> Result<(), MempoolAdapterError>;
}
