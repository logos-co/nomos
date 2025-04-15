use std::fmt::Debug;

use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{RewardsRequestSender, RewardsSenderError},
    RewardMessage,
};

use super::SdpRewardsAdapter;

pub struct LedgerRewardsAdapter<Metadata, ContractAddress> {
    phantom: std::marker::PhantomData<(Metadata, ContractAddress)>,
}

impl<Metadata: Send + Sync, ContractAddress: Send + Debug + Sync> SdpRewardsAdapter
    for LedgerRewardsAdapter<Metadata, ContractAddress>
{
    fn new() -> Self {
        Self {
            phantom: std::marker::PhantomData,
        }
    }
}

impl<Metadata, ContractAddress> LedgerRewardsAdapter<Metadata, ContractAddress> {
    pub fn new() -> Self {
        Self {
            phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<Metadata, ContractAddress> RewardsRequestSender
    for LedgerRewardsAdapter<Metadata, ContractAddress>
where
    Metadata: std::marker::Send + std::marker::Sync,
    ContractAddress: std::marker::Send + std::fmt::Debug + std::marker::Sync,
{
    async fn request_reward(
        &self,
        reward_contract: Self::ContractAddress,
        reward_message: RewardMessage<Self::Metadata>,
    ) -> Result<(), RewardsSenderError<Self::ContractAddress>> {
        todo!() // Implementation will go here later
    }
}
