use async_trait::async_trait;
use chain_service::Slot;
use nomos_ledger::EpochState;
use overwatch::overwatch::OverwatchHandle;

use crate::epoch_info::ChainApi;
use ark_ff::fields::AdditiveGroup;

pub const NON_EXISTING_EPOCH_STATE_SLOT: Slot = Slot::new(4);

pub fn default_epoch_state() -> EpochState {
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

    async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState> {
        if slot == NON_EXISTING_EPOCH_STATE_SLOT {
            None
        } else {
            Some(default_epoch_state())
        }
    }
}
