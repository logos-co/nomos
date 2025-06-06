use nomos_core::block::BlockNumber;
use nomos_sdp_core::ledger::SdpLedgerError;
use overwatch::DynError;
use thiserror::Error;

use crate::adapters::{declaration::SdpDeclarationAdapter, services::SdpServicesAdapter};

pub mod ledger;

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub timestamp: BlockNumber,
}

#[derive(Debug, Error)]
pub enum SdpBackendError {
    #[error("Sdp ledger error: {0}")]
    SdpLedgerError(#[from] SdpLedgerError),

    #[error("Other SDP backend error: {0}")]
    Other(#[from] DynError),
}

#[async_trait::async_trait]
pub trait SdpBackend {
    type Message: Send + Sync;
    type DeclarationAdapter: SdpDeclarationAdapter;
    type ServicesAdapter: SdpServicesAdapter;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        services_adapter: Self::ServicesAdapter,
    ) -> Self;

    async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: Self::Message,
    ) -> Result<(), SdpBackendError>;

    async fn mark_in_block(&mut self, block_number: BlockNumber) -> Result<(), SdpBackendError>;

    fn discard_block(&mut self, block_number: BlockNumber);
}
