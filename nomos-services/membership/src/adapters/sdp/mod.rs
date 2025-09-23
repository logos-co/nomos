pub mod ledger;

use async_trait::async_trait;
use nomos_sdp::FinalizedBlockUpdateStream;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdpAdapterError {
    #[error(transparent)]
    Other(#[from] DynError),
}

#[async_trait]
pub trait SdpAdapter {
    type SdpService: ServiceData;

    fn new(outbound_relay: OutboundRelay<<Self::SdpService as ServiceData>::Message>) -> Self;
    async fn lib_blocks_stream(&self) -> Result<FinalizedBlockUpdateStream, SdpAdapterError>;
}
