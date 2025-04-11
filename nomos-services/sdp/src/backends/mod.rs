use overwatch::DynError;

use crate::adapters::{
    declaration::SdpDeclarationAdapter, rewards::SdpRewardsAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};

pub mod ledger;

#[async_trait::async_trait]
pub trait SdpBackend {
    type BlockNumber: Clone + Send + Sync;
    type Message: Send + Sync;
    type DeclarationAdapter: SdpDeclarationAdapter;
    type RewardsAdapter: SdpRewardsAdapter;
    type StakesVerifierAdapter: SdpStakesVerifierAdapter;
    type ServicesAdapter: SdpServicesAdapter;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        rewards_adapter: Self::RewardsAdapter,
        services_adapter: Self::ServicesAdapter,
        stake_verifier_adapter: Self::StakesVerifierAdapter,
    ) -> Self;

    async fn process_sdp_message(
        &mut self,
        block_number: Self::BlockNumber,
        message: Self::Message,
    ) -> Result<(), DynError>;

    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), DynError>;
    fn discard_block(&mut self, block_number: Self::BlockNumber);
}
