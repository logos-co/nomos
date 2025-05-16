use std::fmt::Debug;

use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{ActivityContract, ActivityContractError},
    ActiveMessage,
};

use super::SdpActivityAdapter;

pub struct LedgerRewardsAdapter<Metadata, ContractAddress> {
    phantom: std::marker::PhantomData<(Metadata, ContractAddress)>,
}

impl<Metadata: Send + Sync, ContractAddress: Send + Debug + Sync> SdpActivityAdapter
    for LedgerRewardsAdapter<Metadata, ContractAddress>
{
    fn new() -> Self {
        Self {
            phantom: std::marker::PhantomData,
        }
    }
}

impl<Metadata, ContractAddress> LedgerRewardsAdapter<Metadata, ContractAddress> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phantom: std::marker::PhantomData,
        }
    }
}

impl Default for LedgerRewardsAdapter<(), ()> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<Metadata, ContractAddress> ActivityContract for LedgerRewardsAdapter<Metadata, ContractAddress>
where
    Metadata: std::marker::Send + std::marker::Sync,
    ContractAddress: std::marker::Send + std::fmt::Debug + std::marker::Sync,
{
    type ContractAddress = ContractAddress;
    type Metadata = Metadata;

    async fn mark_active(
        &self,
        _: Self::ContractAddress,
        _: ActiveMessage<Self::Metadata>,
    ) -> Result<(), ActivityContractError<Self::ContractAddress>> {
        todo!() // Implementation will go here later
    }
}
