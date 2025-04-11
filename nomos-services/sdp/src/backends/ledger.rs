use std::fmt::Debug;

use nomos_sdp_core::{
    ledger::{SdpLedger, ServicesRepository},
    BlockNumber, SdpMessage,
};
use overwatch::DynError;

use super::SdpBackend;
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
    ) -> Result<(), DynError> {
        self.process_sdp_message(block_number, message)
            .await
            .map_err(|e| Box::new(e) as DynError)
    }

    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), DynError> {
        self.mark_in_block(block_number)
            .await
            .map_err(|e| Box::new(e) as DynError)
    }

    fn discard_block(&mut self, block_number: Self::BlockNumber) {
        self.discard_block(block_number);
    }
}
