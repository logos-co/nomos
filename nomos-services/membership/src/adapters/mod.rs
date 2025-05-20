use std::error::Error;

pub mod sdp;

use async_trait::async_trait;
use nomos_sdp::FinalizedBlockUpdateStream;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

pub enum SdpAdapterError {
    Other(DynError),
}

impl<E> From<E> for SdpAdapterError
where
    E: Error + Send + Sync + 'static,
{
    fn from(e: E) -> Self {
        Self::Other(Box::new(e) as DynError)
    }
}

#[async_trait]
pub trait SdpAdapter {
    type SdpService: ServiceData;

    fn new(outbound_relay: OutboundRelay<<Self::SdpService as ServiceData>::Message>) -> Self;
    async fn finalized_blocks_stream(&self) -> Result<FinalizedBlockUpdateStream, SdpAdapterError>;
}
