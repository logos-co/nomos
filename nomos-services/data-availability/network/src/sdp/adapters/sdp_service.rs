use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use async_trait::async_trait;
use nomos_sdp::{
    SdpMessage, SdpService, adapters::mempool::SdpMempoolAdapter as SdpMempoolAdapterTrait,
    backends::SdpBackend,
};
use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, relay::OutboundRelay},
};

use crate::{
    opinion_aggregator::Opinions,
    sdp::{SdpAdapter, SdpAdapterError},
};

pub struct SdpServiceAdapter<Backend, SdpMempoolAdapter, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
{
    relay: OutboundRelay<SdpMessage>,
    _phantom: PhantomData<(Backend, SdpMempoolAdapter, RuntimeServiceId)>,
}

#[async_trait]
impl<Backend, SdpMempoolAdapter, RuntimeServiceId> SdpAdapter<RuntimeServiceId>
    for SdpServiceAdapter<Backend, SdpMempoolAdapter, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    SdpMempoolAdapter: SdpMempoolAdapterTrait<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<SdpService<Backend, SdpMempoolAdapter, RuntimeServiceId>>
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
            .relay::<SdpService<Backend, SdpMempoolAdapter, RuntimeServiceId>>()
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
