use async_trait::async_trait;
use chain_service::Slot;
use nomos_ledger::EpochState;
use overwatch::overwatch::OverwatchHandle;

use crate::epoch_info::ChainApi;

pub fn default_epoch_state() -> EpochState {
    use groth16::Field as _;
    use nomos_core::crypto::ZkHash;
    use nomos_ledger::UtxoTree;

    EpochState {
        epoch: 1.into(),
        nonce: ZkHash::ZERO,
        total_stake: 1_000,
        utxos: UtxoTree::new(),
    }
}

#[derive(Clone)]
pub struct TestChainService;

#[async_trait]
impl<RuntimeServiceId> ChainApi<RuntimeServiceId> for TestChainService {
    async fn new(_: &OverwatchHandle<RuntimeServiceId>) -> Self {
        Self
    }

    async fn get_epoch_state_for_slot(&self, _slot: Slot) -> EpochState {
        default_epoch_state()
    }
}
