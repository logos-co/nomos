use std::marker::PhantomData;

use async_trait::async_trait;
use nomos_sdp::{
    adapters::{
        declaration::repository::SdpDeclarationAdapter,
        services::services_repository::SdpServicesAdapter,
    },
    backends::SdpBackend,
    FinalizedBlockUpdateStream, SdpMessage, SdpService,
};
use overwatch::services::relay::OutboundRelay;
use tokio::sync::oneshot;

use super::{SdpAdapter, SdpAdapterError};

pub struct LedgerSdpAdapter<
    Backend,
    DeclarationAdapter,
    ServicesAdapter,
    Metadata,
    RuntimeServiceId,
> where
    Backend: SdpBackend + Send + Sync + 'static,
{
    relay: OutboundRelay<SdpMessage<Backend>>,
    _phantom_adapters: PhantomData<(DeclarationAdapter, ServicesAdapter)>,

    _phantom_metadata: PhantomData<Metadata>,
    _phantom_runtime_service_id: PhantomData<RuntimeServiceId>,
}

#[async_trait]
impl<Backend, DeclarationAdapter, ServicesAdapter, Metadata, RuntimeServiceId> SdpAdapter
    for LedgerSdpAdapter<Backend, DeclarationAdapter, ServicesAdapter, Metadata, RuntimeServiceId>
where
    Backend: SdpBackend<DeclarationAdapter = DeclarationAdapter, ServicesAdapter = ServicesAdapter>
        + Send
        + Sync
        + 'static,
    DeclarationAdapter: SdpDeclarationAdapter + Send + Sync,
    ServicesAdapter: SdpServicesAdapter + Send + Sync,
    Metadata: Send + Sync + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    type SdpService =
        SdpService<Backend, DeclarationAdapter, ServicesAdapter, Metadata, RuntimeServiceId>;

    fn new(relay: OutboundRelay<SdpMessage<Backend>>) -> Self {
        Self {
            relay,
            _phantom_adapters: PhantomData,
            _phantom_metadata: PhantomData,
            _phantom_runtime_service_id: PhantomData,
        }
    }

    async fn finalized_blocks_stream(&self) -> Result<FinalizedBlockUpdateStream, SdpAdapterError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(SdpMessage::Subscribe {
                result_sender: sender,
            })
            .await
            .map_err(|(e, _)| SdpAdapterError::Other(Box::new(e)))?;

        Ok(receiver.await?)
    }
}
