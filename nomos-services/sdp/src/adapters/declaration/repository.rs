use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{DeclarationsRepository, DeclarationsRepositoryError},
    DeclarationInfo,
};

pub struct LedgerDeclarationAdapter;

pub trait SdpDeclarationAdapter: DeclarationsRepository {
    fn new() -> Self;
}

impl SdpDeclarationAdapter for LedgerDeclarationAdapter {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl DeclarationsRepository for LedgerDeclarationAdapter {
    async fn get(
        &self,
        _id: nomos_sdp_core::DeclarationId,
    ) -> Result<DeclarationInfo, DeclarationsRepositoryError> {
        todo!()
    }

    async fn update(
        &self,
        _declaration_update: DeclarationInfo,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }

    async fn check_nonce(
        &self,
        _declaration_id: nomos_sdp_core::DeclarationId,
        _nonce: nomos_sdp_core::Nonce,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }
}
