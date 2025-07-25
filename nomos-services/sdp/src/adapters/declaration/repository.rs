use async_trait::async_trait;
use nomos_core::sdp::{self, DeclarationInfo};

use crate::backends::{DeclarationsRepository, DeclarationsRepositoryError};

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
        _id: sdp::DeclarationId,
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
        _declaration_id: sdp::DeclarationId,
        _nonce: sdp::Nonce,
    ) -> Result<(), DeclarationsRepositoryError> {
        todo!()
    }
}
