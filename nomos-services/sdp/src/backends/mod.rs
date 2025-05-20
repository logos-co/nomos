use nomos_sdp_core::BlockNumber;
use overwatch::DynError;

use crate::adapters::{
    activity::SdpActivityAdapter, declaration::SdpDeclarationAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};

pub mod ledger;

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub timestamp: BlockNumber,
}

#[derive(Debug)]
pub enum SdpBackendError {
    DeclarationAdapterError(DynError),
    RewardsAdapterError(DynError),
    StakesVerifierAdapterError(DynError),
    ServicesAdapterError(DynError),
    Other(DynError),
}

#[async_trait::async_trait]
pub trait SdpBackend {
    type Message: Send + Sync;
    type DeclarationAdapter: SdpDeclarationAdapter;
    type RewardsAdapter: SdpActivityAdapter;
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
        block_number: BlockNumber,
        message: Self::Message,
    ) -> Result<(), SdpBackendError>;

    async fn mark_in_block(&mut self, block_number: BlockNumber) -> Result<(), SdpBackendError>;

    fn discard_block(&mut self, block_number: BlockNumber);
}
