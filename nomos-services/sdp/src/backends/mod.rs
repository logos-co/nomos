pub mod ledger;

use std::error::Error;

use async_trait::async_trait;
use nomos_core::{
    block::BlockNumber,
    sdp::{
        state::ProviderStateError, DeclarationId, DeclarationInfo, Nonce, ServiceParameters,
        ServiceType,
    },
};
use overwatch::DynError;
use thiserror::Error;

use crate::adapters::{
    declaration::repository::SdpDeclarationAdapter,
    services::services_repository::SdpServicesAdapter,
};

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub timestamp: BlockNumber,
}

#[derive(thiserror::Error, Debug)]
pub enum SdpLedgerError {
    #[error(transparent)]
    ProviderState(#[from] ProviderStateError),
    #[error(transparent)]
    DeclarationsRepository(#[from] DeclarationsRepositoryError),
    #[error(transparent)]
    ServicesRepository(#[from] ServicesRepositoryError),
    #[error("Provider service is already declared in declaration")]
    DuplicateServiceDeclaration,
    #[error("Declaration is not for {0:?} service")]
    IncorrectServiceType(ServiceType),
    #[error("Duplicate declaration for provider it in block")]
    DuplicateDeclarationInBlock,
    #[error("Provider declaration id and message declaration id does not match")]
    WrongDeclarationId,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(thiserror::Error, Debug)]
pub enum DeclarationsRepositoryError {
    #[error("Declaration not found: {0:?}")]
    DeclarationNotFound(DeclarationId),
    #[error("Duplicate nonce")]
    DuplicateNonce,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait DeclarationsRepository {
    async fn get(&self, id: DeclarationId) -> Result<DeclarationInfo, DeclarationsRepositoryError>;
    async fn update(&self, declaration: DeclarationInfo)
        -> Result<(), DeclarationsRepositoryError>;
    async fn check_nonce(
        &self,
        declaration_id: DeclarationId,
        nonce: Nonce,
    ) -> Result<(), DeclarationsRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum ServicesRepositoryError {
    #[error("Service not found: {0:?}")]
    NotFound(ServiceType),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait ServicesRepository {
    async fn get_parameters(
        &self,
        service_type: ServiceType,
    ) -> Result<ServiceParameters, ServicesRepositoryError>;
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
