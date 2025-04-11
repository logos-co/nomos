use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{self, DeclarationsRepositoryError},
    Declaration, ProviderInfo,
};

use super::SdpDeclarationAdapter;
pub struct LedgerDeclarationAdapter;

impl SdpDeclarationAdapter for LedgerDeclarationAdapter {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ledger::DeclarationsRepository for LedgerDeclarationAdapter {
    async fn get_provider_info(
        &self,
        _provider_id: nomos_sdp_core::ProviderId,
    ) -> Result<ProviderInfo, DeclarationsRepositoryError> {
        todo!()
    }

    async fn get_declaration(
        &self,
        _declaration_id: nomos_sdp_core::DeclarationId,
    ) -> Result<Declaration, DeclarationsRepositoryError> {
        todo!()
    }

    async fn update_provider_info(
        &self,
        _provider_info: ProviderInfo,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }

    async fn update_declaration(
        &self,
        _declaration_update: nomos_sdp_core::DeclarationUpdate,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }

    async fn check_nonce(
        &self,
        _provider_id: nomos_sdp_core::ProviderId,
        _nonce: nomos_sdp_core::Nonce,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }
}
