pub mod sdp_service;

use async_trait::async_trait;
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
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
    async fn post_activity(&self, metadata: Vec<u8>) -> Result<(), SdpAdapterError>;
}
