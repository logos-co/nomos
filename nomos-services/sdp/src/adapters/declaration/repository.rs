use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{self, DeclarationsRepositoryError},
    DeclarationInfo,
};

pub struct LedgerDeclarationAdapter;

impl SdpDeclarationAdapter for LedgerDeclarationAdapter {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ledger::DeclarationsRepository for LedgerDeclarationAdapter {
    async fn get(
        &self,
        _id: nomos_sdp_core::DeclarationId,
    ) -> Result<DeclarationInfo, DeclarationsRepositoryError> {
        todo!()
    }

    async fn update(
        &self,
        _declaration_update: nomos_sdp_core::DeclarationInfo,
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

pub trait SdpDeclarationAdapter: ledger::DeclarationsRepository {
    fn new() -> Self;
}
