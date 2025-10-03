use core::{cmp::Ordering, fmt::Debug, marker::PhantomData};

use async_trait::async_trait;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use cryptarchia_engine::{Epoch, Slot};
use futures::Stream;
use nomos_blend_message::crypto::proofs::quota::inputs::prove::private::ProofOfLeadershipQuotaInputs;
use nomos_core::crypto::ZkHash;
use nomos_ledger::EpochState;
use nomos_time::SlotTick;
use overwatch::overwatch::OverwatchHandle;

#[derive(Clone, Debug)]
pub struct PolEpochInfo {
    pub epoch: Epoch,
    pub poq_private_inputs: ProofOfLeadershipQuotaInputs,
}

#[async_trait]
pub trait PolInfoProvider<RuntimeServiceId> {
    type Stream: Stream<Item = PolEpochInfo>;

    async fn subscribe(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Option<Self::Stream>;
}

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
pub struct EpochHandler<ChainService, RuntimeServiceId> {
    chain_service: ChainService,
    last_processed_tick: Option<SlotTick>,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<ChainService, RuntimeServiceId> EpochHandler<ChainService, RuntimeServiceId> {
    pub const fn new(chain_service: ChainService) -> Self {
        Self {
            chain_service,
            last_processed_tick: None,
            _phantom: PhantomData,
        }
    }
}

impl<ChainService, RuntimeServiceId> EpochHandler<ChainService, RuntimeServiceId>
where
    ChainService: ChainApi<RuntimeServiceId>,
{
    pub async fn tick(
        &mut self,
        SlotTick {
            epoch: new_epoch,
            slot: new_slot,
        }: SlotTick,
    ) -> Option<EpochInfo> {
        if let Some(SlotTick { epoch, slot }) = self.last_processed_tick {
            match (epoch.cmp(&new_epoch), slot.cmp(&new_slot)) {
                // Bail early if epoch is smaller or slot is not strictly larger.
                (Ordering::Greater, _) | (_, Ordering::Equal | Ordering::Greater) => {
                    tracing::error!(target: LOG_TARGET, "Slot ticks are assumed to be always increasing for both epochs and slots.");
                    return None;
                }
                // Ignore if epoch has not changed.
                (Ordering::Equal, _) => {
                    tracing::trace!(target: LOG_TARGET, "New slot for same epoch. Skipping...");
                    self.last_processed_tick = Some(SlotTick {
                        epoch: new_epoch,
                        slot: new_slot,
                    });
                    return None;
                }
                (Ordering::Less, Ordering::Less) => {}
            }
        }

        tracing::debug!(target: LOG_TARGET, "Found new epoch unseen before. Polling for its state...");
        self.fetch_and_process_epoch_info(SlotTick {
            epoch: new_epoch,
            slot: new_slot,
        })
        .await
        .map(Into::into)
    }

    async fn fetch_and_process_epoch_info(
        &mut self,
        SlotTick { epoch, slot }: SlotTick,
    ) -> Option<EpochState> {
        let Some(epoch_state_for_slot) = self.chain_service.get_epoch_state_for_slot(slot).await
        else {
            tracing::warn!(target: LOG_TARGET, "No epoch state for given slot. Retrying on the next slot tick.");
            return None;
        };

        self.last_processed_tick = Some(SlotTick { epoch, slot });
        Some(epoch_state_for_slot)
    }
}

#[cfg(test)]
mod tests {

    use nomos_time::SlotTick;
    use test_log::test;

    use crate::{
        epoch_info::EpochHandler,
        test_utils::epoch::{NON_EXISTING_EPOCH_STATE_SLOT, TestChainService, default_epoch_state},
    };

    type TestEpochHandler = EpochHandler<TestChainService, ()>;

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
        let mut stream = TestEpochHandler::new(TestChainService);

        // First poll of the stream will set the epoch info and return the retrieved
        // state.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 1.into(),
                slot: 1.into()
            })
        );
        assert_eq!(next_tick, Some(default_epoch_state().into()));

        // Second poll of the stream will not return anything since it's in the same
        // epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert!(next_tick.is_none());
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 1.into(),
                slot: 2.into()
            })
        );

        // Third poll of the stream will yield a new element since we're in a new epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into()
            })
        );
        assert_eq!(next_tick, Some(default_epoch_state().into()));

        // Fourth poll of the stream will not yield anything since there was no state
        // for the new epoch, and the epoch info is not updated.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into()
            })
        );
        assert!(next_tick.is_none());

        // Fifth poll updates epoch and return state as expected.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 3.into(),
                slot: 5.into()
            })
        );
        assert_eq!(next_tick, Some(default_epoch_state().into()));
    }

    #[test(tokio::test)]
    async fn slot_not_increasing() {
        let mut stream = TestEpochHandler::new(TestChainService);
        stream
            .tick(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
            .await;
        assert!(
            stream
                .tick(SlotTick {
                    epoch: 3.into(),
                    slot: 2.into(),
                })
                .await
                .is_none()
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
        );
        assert!(
            stream
                .tick(SlotTick {
                    epoch: 3.into(),
                    slot: 1.into(),
                })
                .await
                .is_none()
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
        );
    }

    #[test(tokio::test)]
    async fn epoch_not_increasing() {
        let mut stream = TestEpochHandler::new(TestChainService);
        stream
            .tick(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
            .await;
        assert!(
            stream
                .tick(SlotTick {
                    epoch: 2.into(),
                    slot: 3.into(),
                })
                .await
                .is_none()
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into(),
            })
        );
        assert!(
            stream
                .tick(SlotTick {
                    epoch: 1.into(),
                    slot: 3.into(),
                })
                .await
                .is_none()
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into(),
            })
        );
    }
}
