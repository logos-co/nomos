use async_trait::async_trait;
use nomos_sdp_core::{
    ledger::{self, StakesVerifierError},
    ProviderId,
};

use super::SdpStakesVerifierAdapter;

pub struct LedgerStakesVerifierAdapter<Proof> {
    _proof: std::marker::PhantomData<Proof>,
}

impl<Proof: Send + Sync> SdpStakesVerifierAdapter for LedgerStakesVerifierAdapter<Proof> {
    fn new() -> Self {
        Self {
            _proof: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<Proof: std::marker::Send + std::marker::Sync> ledger::StakesVerifier
    for LedgerStakesVerifierAdapter<Proof>
{
    type Proof = Proof;

    async fn verify(
        &self,
        provider_id: ProviderId,
        proof: Self::Proof,
    ) -> Result<(), StakesVerifierError> {
        todo!() // Implementation will go here later
    }
}
