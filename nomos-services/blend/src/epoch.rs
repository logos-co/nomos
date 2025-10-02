use core::{fmt::Debug, marker::PhantomData};

use async_trait::async_trait;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use cryptarchia_engine::{Epoch, Slot};
use nomos_core::crypto::ZkHash;
use nomos_ledger::EpochState;
use nomos_time::SlotTick;
use overwatch::overwatch::OverwatchHandle;

const LOG_TARGET: &str = "blend::service::epoch";

#[derive(Debug, PartialEq, Eq)]
pub struct EpochInfo {
    nonce: ZkHash,
    ledger_aged: ZkHash,
    total_stake: u64,
}

impl From<EpochState> for EpochInfo {
    fn from(
        EpochState {
            total_stake,
            utxos,
            nonce,
            ..
        }: EpochState,
    ) -> Self {
        Self {
            ledger_aged: utxos.root(),
            nonce,
            total_stake,
        }
    }
}

/// A trait that provides the needed functionalities for the epoch stream to
/// fetch the epoch state for a given slot.
#[async_trait]
pub trait ChainApi<RuntimeServiceId> {
    async fn new(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) -> Self;
    async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState>;
}

#[async_trait]
impl<Cryptarchia, RuntimeServiceId> ChainApi<RuntimeServiceId>
    for CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    RuntimeServiceId: Send + Sync,
{
    async fn new(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) -> Self {
        Self::new(overwatch_handle).await
    }

    async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState> {
        self.get_epoch_state(slot)
            .await
            .expect("Failed to get epoch state for slot.")
    }
}

/// A stream that listens to slot ticks, and on the first slot tick received as
/// well as the first slot tick of each new epoch, fetches the epoch state from
/// the provided chain service adapter.
///
/// In case the epoch state for a given slot is not found, it will retry on
/// subsequent slots until one is successfully received.
pub struct EpochStream<ChainService, RuntimeServiceId> {
    chain_service: ChainService,
    current_epoch: Option<Epoch>,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<ChainService, RuntimeServiceId> EpochStream<ChainService, RuntimeServiceId> {
    pub const fn new(chain_service: ChainService) -> Self {
        Self {
            chain_service,
            current_epoch: None,
            _phantom: PhantomData,
        }
    }
}

impl<ChainService, RuntimeServiceId> EpochStream<ChainService, RuntimeServiceId>
where
    ChainService: ChainApi<RuntimeServiceId>,
{
    pub async fn tick(&mut self, SlotTick { epoch, slot }: SlotTick) -> Option<EpochInfo> {
        if let Some(last_processed_epoch) = self.current_epoch
            && last_processed_epoch == epoch
        {
            tracing::debug!(target: LOG_TARGET, "New slot for current epoch. Skipping...");
            return None;
        }

        tracing::debug!(target: LOG_TARGET, "Found new epoch unseen before. Polling for its state...");
        let Some(epoch_state_for_slot) = self.chain_service.get_epoch_state_for_slot(slot).await
        else {
            tracing::warn!(target: LOG_TARGET, "No epoch state for given slot. Retrying on the next slot tick.");
            return None;
        };

        self.current_epoch = Some(epoch);
        Some(epoch_state_for_slot.into())
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use cryptarchia_engine::Slot;
    use nomos_core::crypto::ZkHash;
    use nomos_ledger::{EpochState, UtxoTree};
    use nomos_time::SlotTick;
    use overwatch::overwatch::OverwatchHandle;
    use test_log::test;

    use crate::epoch::{ChainApi, EpochStream};

    const NON_EXISTING_EPOCH_STATE_SLOT: Slot = Slot::new(4);

    fn default_epoch_state() -> EpochState {
        use groth16::Field as _;

        EpochState {
            epoch: 1.into(),
            nonce: ZkHash::ZERO,
            total_stake: 1_000,
            utxos: UtxoTree::new(),
        }
    }

    #[derive(Clone)]
    struct ChainService;

    #[async_trait]
    impl ChainApi<()> for ChainService {
        async fn new(_: &OverwatchHandle<()>) -> Self {
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

    #[test(tokio::test)]
    async fn epoch_transition() {
        let ticks = vec![
            SlotTick {
                epoch: 1.into(),
                slot: 1.into(),
            },
            // New slot same epoch
            SlotTick {
                epoch: 1.into(),
                slot: 2.into(),
            },
            // New slot new epoch
            SlotTick {
                epoch: 2.into(),
                slot: 3.into(),
            },
            // New slot new epoch, but no associated epoch state
            SlotTick {
                epoch: 3.into(),
                slot: NON_EXISTING_EPOCH_STATE_SLOT,
            },
            // New slot same epoch, but with associated epoch state
            SlotTick {
                epoch: 3.into(),
                slot: 5.into(),
            },
        ];
        let mut ticks_iter = ticks.into_iter();
        let mut stream = EpochStream::new(ChainService);

        // First poll of the stream will set the epoch info and return the retrieved
        // state.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(stream.current_epoch, Some(1.into()));
        assert_eq!(next_tick, Some(default_epoch_state().into()));

        // Second poll of the stream will not return anything since it's in the same
        // epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert!(next_tick.is_none());
        assert_eq!(stream.current_epoch, Some(1.into()));

        // Third poll of the stream will yield a new element since we're in a new epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(stream.current_epoch, Some(2.into()));
        assert_eq!(next_tick, Some(default_epoch_state().into()));

        // Fourth poll of the stream will not yield anything since there was no state
        // for the new epoch, and the epoch info is not updated.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(stream.current_epoch, Some(2.into()));
        assert!(next_tick.is_none());

        // Fifth poll updates epoch and return state as expected.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(stream.current_epoch, Some(3.into()));
        assert_eq!(next_tick, Some(default_epoch_state().into()));
    }
}
