use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use async_trait::async_trait;
use nomos_sdp::{SdpMessage, SdpService, backends::SdpBackend};
use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, relay::OutboundRelay},
};

use crate::{
    opinion_aggregator::Opinions,
    sdp::{SdpAdapter, SdpAdapterError},
};

pub struct SdpServiceAdapter<Backend, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
{
    relay: OutboundRelay<SdpMessage>,
    _phantom: PhantomData<(Backend, RuntimeServiceId)>,
}

#[async_trait]
impl<Backend, RuntimeServiceId> SdpAdapter<RuntimeServiceId>
    for SdpServiceAdapter<Backend, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<SdpService<Backend, RuntimeServiceId>>
        + Send
        + Sync
        + Debug
        + Display
        + 'static,
{
    async fn new(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Result<Self, SdpAdapterError> {
        let relay = overwatch_handle
            .relay::<SdpService<Backend, RuntimeServiceId>>()
            .await
            .map_err(|e| SdpAdapterError::Other(Box::new(e)))?;

        Ok(Self {
            relay,
            _phantom: PhantomData,
        })
    }

    async fn post_activity(&self, opinions: Opinions) -> Result<(), SdpAdapterError> {
        let metadata = opinions.into();
        self.relay
            .send(SdpMessage::PostActivity { metadata })
            .await
            .map_err(|(e, _)| SdpAdapterError::Other(Box::new(e)))?;

        Ok(())
    }
}
