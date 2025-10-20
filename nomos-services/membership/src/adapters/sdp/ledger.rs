use std::marker::PhantomData;

use async_trait::async_trait;
use nomos_sdp::{BlockUpdateStream, SdpMessage, SdpService, backends::SdpBackend};
use overwatch::services::relay::OutboundRelay;
use tokio::sync::oneshot;

use super::{SdpAdapter, SdpAdapterError};

pub struct LedgerSdpAdapter<Backend, Metadata, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
{
    relay: OutboundRelay<SdpMessage>,
    _phantom: PhantomData<(Backend, Metadata, RuntimeServiceId)>,
}

#[async_trait]
impl<Backend, Metadata, RuntimeServiceId> SdpAdapter
    for LedgerSdpAdapter<Backend, Metadata, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    type SdpService = SdpService<Backend, Metadata, RuntimeServiceId>;

    fn new(relay: OutboundRelay<SdpMessage>) -> Self {
        Self {
            relay,
            _phantom: PhantomData,
        }
    }

    async fn lib_blocks_stream(&self) -> Result<BlockUpdateStream, SdpAdapterError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(SdpMessage::Subscribe {
                result_sender: sender,
            })
            .await
            .map_err(|(e, _)| SdpAdapterError::Other(Box::new(e)))?;

        Ok(receiver
            .await
            .map_err(|e| SdpAdapterError::Other(Box::new(e)))?)
    }
}
