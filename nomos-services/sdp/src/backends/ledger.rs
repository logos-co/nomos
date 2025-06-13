use nomos_core::block::BlockNumber;
use nomos_sdp_core::{ledger::SdpLedger, SdpMessage};

use super::{SdpBackend, SdpBackendError};
use crate::adapters::{
    declaration::repository::SdpDeclarationAdapter,
    services::services_repository::SdpServicesAdapter,
};

#[async_trait::async_trait]
impl<Declarations, Services, Metadata> SdpBackend for SdpLedger<Declarations, Services, Metadata>
where
    Metadata: Send + Sync + 'static,
    Declarations: SdpDeclarationAdapter + Send + Sync,
    Services: SdpServicesAdapter + Send + Sync,
{
    type Message = SdpMessage<Metadata>;
    type DeclarationAdapter = Declarations;
    type ServicesAdapter = Services;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        services_adapter: Self::ServicesAdapter,
    ) -> Self {
        Self::new(declaration_adapter, services_adapter)
    }

    async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: Self::Message,
    ) -> Result<(), SdpBackendError> {
        self.process_sdp_message(block_number, message)
            .await
            .map_err(Into::into)
    }

    async fn mark_in_block(&mut self, block_number: BlockNumber) -> Result<(), SdpBackendError> {
        self.mark_in_block(block_number).await.map_err(Into::into)
    }

    fn discard_block(&mut self, block_number: BlockNumber) {
        self.discard_block(block_number);
    }
}
