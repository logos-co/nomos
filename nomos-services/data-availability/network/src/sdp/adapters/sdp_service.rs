use std::marker::PhantomData;

use async_trait::async_trait;
use nomos_sdp::{SdpMessage, SdpService, backends::SdpBackend};
use overwatch::services::relay::OutboundRelay;

use super::{SdpAdapter, SdpAdapterError};
use crate::opinion_aggregator::ActivityProof;

pub struct SdpServiceAdapter<Backend, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
{
    relay: OutboundRelay<SdpMessage>,
    _phantom: PhantomData<(Backend, RuntimeServiceId)>,
}

#[async_trait]
impl<Backend, RuntimeServiceId> SdpAdapter for SdpServiceAdapter<Backend, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    type SdpService = SdpService<Backend, RuntimeServiceId>;

    fn new(relay: OutboundRelay<SdpMessage>) -> Self {
        Self {
            relay,
            _phantom: PhantomData,
        }
    }

    async fn post_activity(&self, activity_proof: ActivityProof) -> Result<(), SdpAdapterError> {
        let metadata = activity_proof.to_metadata_bytes();
        self.relay
            .send(SdpMessage::PostActivity { metadata })
            .await
            .map_err(|(e, _)| SdpAdapterError::Other(Box::new(e)))?;

        Ok(())
    }
}
