use nomos_sdp_core::{
    ledger::{
        SdpLedger,
        SdpLedgerError::{
            self, DeclarationsRepository, DuplicateDeclarationInBlock, DuplicateServiceDeclaration,
            Other, ProviderState, ServiceNotProvided,
            ServicesRepository as LedgerServicesRepository, StakesVerifier, WrongDeclarationId,
        },
        ServicesRepository,
    },
    BlockNumber, SdpMessage,
};

use super::{SdpBackend, SdpBackendError};
use crate::adapters::{
    declaration::SdpDeclarationAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};

#[async_trait::async_trait]
impl<Declarations, Services, Stakes, Proof, Metadata> SdpBackend
    for SdpLedger<Declarations, Services, Stakes, Proof, Metadata>
where
    Services: ServicesRepository + Send + Sync + Clone + 'static,
    Proof: Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
    Declarations: SdpDeclarationAdapter + Send + Sync,
    Stakes: SdpStakesVerifierAdapter<Proof = Proof> + Send + Sync,
    Services: SdpServicesAdapter + Send + Sync,
{
    type BlockNumber = BlockNumber;
    type Message = SdpMessage<Metadata, Proof>;
    type DeclarationAdapter = Declarations;
    type ServicesAdapter = Services;
    type StakesVerifierAdapter = Stakes;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        services_adapter: Self::ServicesAdapter,
        stake_verifier_adapter: Self::StakesVerifierAdapter,
    ) -> Self {
        Self::new(
            declaration_adapter,
            services_adapter,
            stake_verifier_adapter,
        )
    }

    async fn process_sdp_message(
        &mut self,
        block_number: Self::BlockNumber,
        message: Self::Message,
    ) -> Result<(), SdpBackendError> {
        self.process_sdp_message(block_number, message)
            .await
            .map_err(Into::into)
    }

    async fn mark_in_block(
        &mut self,
        block_number: Self::BlockNumber,
    ) -> Result<(), SdpBackendError> {
        self.mark_in_block(block_number).await.map_err(Into::into)
    }

    fn discard_block(&mut self, block_number: Self::BlockNumber) {
        self.discard_block(block_number);
    }
}

impl From<SdpLedgerError> for SdpBackendError {
    fn from(e: SdpLedgerError) -> Self {
        match e {
            ProviderState(provider_state_error) => Self::Other(Box::new(provider_state_error)),
            DeclarationsRepository(err) => Self::DeclarationAdapterError(Box::new(err)),
            LedgerServicesRepository(err) => Self::ServicesAdapterError(Box::new(err)),
            StakesVerifier(err) => Self::StakesVerifierAdapterError(Box::new(err)),
            DuplicateServiceDeclaration
            | ServiceNotProvided(_)
            | DuplicateDeclarationInBlock
            | WrongDeclarationId => Self::Other(Box::new(e)),
            Other(error) => Self::Other(error),
        }
    }
}
