pub mod sdp;

use std::error::Error;

use nomos_sdp::{backends::SdpBackendError, FinalizedBlockUpdateStream};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

pub enum SdpAdapterError {
    SdpBackend(SdpBackendError),
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

pub trait SdpAdapter {
    type SdpService: ServiceData;

    fn new(outbound_relay: OutboundRelay<<Self::SdpService as ServiceData>::Message>) -> Self;
    async fn subscribe(&self) -> Result<FinalizedBlockUpdateStream, SdpAdapterError>;
}
