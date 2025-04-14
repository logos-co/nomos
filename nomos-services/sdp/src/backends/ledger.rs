use std::fmt::Debug;

use nomos_sdp_core::{
    ledger::{
        SdpLedger,
        SdpLedgerError::{
            self, DeclarationsRepository, DuplicateDeclarationInBlock, DuplicateServiceDeclaration,
            Other, ProviderState, RewardsSender, ServiceNotProvided,
            ServicesRepository as LedgerServicesRepository, StakesVerifier, WrongDeclarationId,
        },
        ServicesRepository,
    },
    BlockNumber, SdpMessage,
};

use super::{SdpBackend, SdpBackendError};
use crate::adapters::{
    declaration::SdpDeclarationAdapter, rewards::SdpRewardsAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};

#[async_trait::async_trait]
impl<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress> SdpBackend
    for SdpLedger<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress>
where
    Services: ServicesRepository<ContractAddress = ContractAddress> + Send + Sync + Clone + 'static,
    ContractAddress: Debug + Send + Sync + 'static,
    Proof: Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
    Declarations: SdpDeclarationAdapter + Send + Sync,
    Rewards:
        SdpRewardsAdapter<Metadata = Metadata, ContractAddress = ContractAddress> + Send + Sync,
    Stakes: SdpStakesVerifierAdapter<Proof = Proof> + Send + Sync,
    Services: SdpServicesAdapter + Send + Sync,
{
    type BlockNumber = BlockNumber;
    type Message = SdpMessage<Metadata, Proof>;
    type DeclarationAdapter = Declarations;
    type ServicesAdapter = Services;
    type RewardsAdapter = Rewards;
    type StakesVerifierAdapter = Stakes;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        rewards_adapter: Self::RewardsAdapter,
        services_adapter: Self::ServicesAdapter,
        stake_verifier_adapter: Self::StakesVerifierAdapter,
    ) -> Self {
        Self::new(
            declaration_adapter,
            rewards_adapter,
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
            .map_err(map_ledger_error)
    }

    async fn mark_in_block(
        &mut self,
        block_number: Self::BlockNumber,
    ) -> Result<(), SdpBackendError> {
        self.mark_in_block(block_number)
            .await
            .map_err(map_ledger_error)
    }

    fn discard_block(&mut self, block_number: Self::BlockNumber) {
        self.discard_block(block_number);
    }
}

fn map_ledger_error<ContractAddress: Debug + Send + Sync + 'static>(
    e: SdpLedgerError<ContractAddress>,
) -> SdpBackendError {
    match e {
        ProviderState(provider_state_error) => {
            SdpBackendError::Other(Box::new(provider_state_error))
        }
        DeclarationsRepository(err) => SdpBackendError::DeclarationAdapterError(Box::new(err)),
        RewardsSender(err) => SdpBackendError::RewardsAdapterError(Box::new(err)),
        LedgerServicesRepository(err) => SdpBackendError::ServicesAdapterError(Box::new(err)),
        StakesVerifier(err) => SdpBackendError::StakesVerifierAdapterError(Box::new(err)),
        DuplicateServiceDeclaration | ServiceNotProvided(_) | DuplicateDeclarationInBlock => {
            SdpBackendError::Other(Box::new(e))
        }
        WrongDeclarationId => SdpBackendError::Other(Box::new(Box::new(e))),
        Other(error) => SdpBackendError::Other(error),
    }
}
