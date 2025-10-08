use core::{cmp::Ordering, fmt::Debug, marker::PhantomData, num::NonZeroU64};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochEvent {
    /// A new epoch is available, which is either ongoing for a while or has
    /// just started.
    NewEpoch(EpochInfo),
    /// The previous epoch which was caught before it finished can now be
    /// discarded since its transition period has elapsed.
    OldEpochTransitionPeriodExpired,
}

impl From<EpochInfo> for EpochEvent {
    fn from(value: EpochInfo) -> Self {
        Self::NewEpoch(value)
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
    last_successfully_processed_epoch: Option<Epoch>,
    last_processed_tick: Option<SlotTick>,
    previous_epoch_number_and_last_slot: Option<(Epoch, Slot)>,
    epoch_transition_period_in_slots: NonZeroU64,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<ChainService, RuntimeServiceId> EpochHandler<ChainService, RuntimeServiceId> {
    pub const fn new(
        chain_service: ChainService,
        epoch_transition_period_in_slots: NonZeroU64,
    ) -> Self {
        Self {
            chain_service,
            last_successfully_processed_epoch: None,
            last_processed_tick: None,
            previous_epoch_number_and_last_slot: None,
            epoch_transition_period_in_slots,
            _phantom: PhantomData,
        }
    }
}

impl<ChainService, RuntimeServiceId> EpochHandler<ChainService, RuntimeServiceId>
where
    ChainService: ChainApi<RuntimeServiceId> + Sync,
    RuntimeServiceId: Sync,
{
    pub async fn tick(&mut self, new_tick: SlotTick) -> Option<EpochEvent> {
        self.validate_new_tick(new_tick).ok()?;

        // If we have witnessed a new epoch being transitioned and we have notified
        // consumers of a new epoch that has started, and we have passed its
        // transition period, notify the consumers.
        if self.check_and_consume_past_epoch_transition_period(new_tick.slot) {
            return Some(EpochEvent::OldEpochTransitionPeriodExpired);
        }

        if let Some(last_successfully_processed_epoch) = self.last_successfully_processed_epoch
            && last_successfully_processed_epoch == new_tick.epoch
        {
            tracing::trace!(target: LOG_TARGET, "New slot for same epoch. Skipping...");
            return None;
        }

        tracing::debug!(target: LOG_TARGET, "Found epoch unseen before. Polling for its state...");
        // If we have already previously processed an epoch, keep track of its last
        // slot, so we can signal consumer when the transition period is over,
        // regardless of whether we are able to notify them about a new epoch that has
        // started.
        if let Some(last_successfully_processed_epoch) = self.last_successfully_processed_epoch {
            self.previous_epoch_number_and_last_slot = Some((
                last_successfully_processed_epoch,
                Slot::new(new_tick.slot.into_inner().saturating_sub(1)),
            ));
        }
        let epoch_state = self.fetch_and_process_epoch_info(new_tick.slot).await?;
        self.last_successfully_processed_epoch = Some(new_tick.epoch);
        Some(EpochInfo::from(epoch_state).into())
    }

    // Input sanity check. Aka we bail early if input is trash, like epoch is
    // smaller or slot is not strictly larger.
    fn validate_new_tick(&mut self, SlotTick { epoch, slot }: SlotTick) -> Result<(), ()> {
        if let Some(SlotTick {
            epoch: last_processed_epoch,
            slot: last_processed_slot,
        }) = self.last_processed_tick
            && (last_processed_epoch > epoch || last_processed_slot >= slot)
        {
            tracing::error!(target: LOG_TARGET, "Slot ticks are assumed to be always increasing for both epochs and slots.");
            return Err(());
        }
        // Else, update the internal clock.
        self.last_processed_tick = Some(SlotTick { epoch, slot });
        Ok(())
    }

    // If we have witnessed a new epoch being transitioned, we have passed its
    // transition period AND a new epoch has been notified, notify the consumers.
    fn check_and_consume_past_epoch_transition_period(&mut self, new_slot: Slot) -> bool {
        if let Some((previous_epoch, previous_epoch_last_slot)) =
            self.previous_epoch_number_and_last_slot
            && new_slot.into_inner() - previous_epoch_last_slot.into_inner()
                >= self.epoch_transition_period_in_slots.get()
            && Some(previous_epoch) != self.last_successfully_processed_epoch
        {
            self.previous_epoch_number_and_last_slot = None;
            return true;
        }
        false
    }

    async fn fetch_and_process_epoch_info(&self, slot: Slot) -> Option<EpochState> {
        let Some(epoch_state_for_slot) = self.chain_service.get_epoch_state_for_slot(slot).await
        else {
            tracing::warn!(target: LOG_TARGET, "No epoch state for given slot. Retrying on the next slot tick.");
            return None;
        };

        Some(epoch_state_for_slot)
    }
}

#[cfg(test)]
mod tests {

    use nomos_time::SlotTick;
    use test_log::test;

    use crate::{
        epoch_info::{EpochHandler, EpochInfo},
        test_utils::epoch::{NON_EXISTING_EPOCH_STATE_SLOT, TestChainService, default_epoch_state},
    };

    type TestEpochHandler = EpochHandler<TestChainService, ()>;

    #[test(tokio::test)]
    async fn epoch_ticks() {
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
        let mut stream = TestEpochHandler::new(TestChainService, u64::MAX.try_into().unwrap());

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
        assert_eq!(
            next_tick,
            Some(EpochInfo::from(default_epoch_state()).into())
        );
        assert_eq!(stream.last_successfully_processed_epoch, Some(1.into()));
        assert!(stream.previous_epoch_number_and_last_slot.is_none());

        // Second poll of the stream will not return anything since it's in the same
        // epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 1.into(),
                slot: 2.into()
            })
        );
        assert!(next_tick.is_none());
        assert_eq!(stream.last_successfully_processed_epoch, Some(1.into()));
        assert!(stream.previous_epoch_number_and_last_slot.is_none());

        // Third poll of the stream will yield a new element since we're in a new epoch.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into()
            })
        );
        assert_eq!(
            next_tick,
            Some(EpochInfo::from(default_epoch_state()).into())
        );
        assert_eq!(stream.last_successfully_processed_epoch, Some(2.into()));
        assert_eq!(
            stream.previous_epoch_number_and_last_slot,
            Some((1.into(), 2.into()))
        );

        // Fourth poll of the stream will not yield anything since there was no state
        // for the new epoch, and the epoch info is not updated.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 3.into(),
                slot: NON_EXISTING_EPOCH_STATE_SLOT
            })
        );
        assert!(next_tick.is_none());
        assert_eq!(stream.last_successfully_processed_epoch, Some(2.into()));
        assert_eq!(
            stream.previous_epoch_number_and_last_slot,
            Some((2.into(), 3.into()))
        );

        // Fifth poll updates epoch and return state as expected.
        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 3.into(),
                slot: 5.into()
            })
        );
        assert_eq!(
            next_tick,
            Some(EpochInfo::from(default_epoch_state()).into())
        );
        assert_eq!(stream.last_successfully_processed_epoch, Some(3.into()));
        // This implies that the previous epoch lasted until the first slot of the new
        // epoch, because its state could not be fetched, hence we held off notifying
        // consumers about the event until we had a replacement epoch.
        // This is a very niche edge case, and in practice it will never happen.
        assert_eq!(
            stream.previous_epoch_number_and_last_slot,
            Some((2.into(), NON_EXISTING_EPOCH_STATE_SLOT))
        );
    }

    #[test(tokio::test)]
    async fn epoch_transition() {
        unimplemented!("Continue from here");
        let ticks = vec![
            // Ongoing epoch
            SlotTick {
                epoch: 1.into(),
                slot: 1.into(),
            },
            // New slot same epoch
            SlotTick {
                epoch: 1.into(),
                slot: 2.into(),
            },
            // Third slot same epoch
            SlotTick {
                epoch: 1.into(),
                slot: 3.into(),
            },
            // New slot new epoch, but no associated epoch state
            SlotTick {
                epoch: 2.into(),
                slot: NON_EXISTING_EPOCH_STATE_SLOT,
            },
            // New slot same epoch
            SlotTick {
                epoch: 2.into(),
                slot: 5.into(),
            },
            // New slot new epoch
            SlotTick {
                epoch: 3.into(),
                slot: 6.into(),
            },
        ];
        let mut ticks_iter = ticks.into_iter();
        let mut stream = TestEpochHandler::new(TestChainService, 1.try_into().unwrap());

        let next_tick = stream.tick(ticks_iter.next().unwrap()).await;
        assert!(next_tick.is_none());
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 1.into(),
                slot: 2.into()
            })
        );
    }

    #[test(tokio::test)]
    async fn slot_not_increasing() {
        let mut stream = TestEpochHandler::new(TestChainService, u64::MAX.try_into().unwrap());
        stream
            .tick(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
            .await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into()
            })
        );
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
                slot: 2.into()
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into()
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
                slot: 2.into()
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into()
            })
        );
    }

    #[test(tokio::test)]
    async fn epoch_not_increasing() {
        let mut stream = TestEpochHandler::new(TestChainService, u64::MAX.try_into().unwrap());
        stream
            .tick(SlotTick {
                epoch: 2.into(),
                slot: 2.into(),
            })
            .await;
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 2.into()
            })
        );
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
                slot: 3.into()
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into(),
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into()
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
                slot: 3.into()
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into(),
            })
        );
        assert_eq!(
            stream.last_processed_tick,
            Some(SlotTick {
                epoch: 2.into(),
                slot: 3.into()
            })
        );
    }
}
