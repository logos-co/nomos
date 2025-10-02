use core::{
    fmt::Debug,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use cryptarchia_engine::Epoch;
use futures::{FutureExt as _, Stream, StreamExt as _};
use nomos_core::crypto::ZkHash;
use nomos_ledger::EpochState;
use nomos_time::SlotTick;

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

type GetEpochForSlotFuture = Pin<Box<dyn Future<Output = Option<EpochState>> + Send>>;

/// A stream that listens to slot ticks, and on the first slot tick received as
/// well as the first slot tick of each new epoch, fetches the epoch state from
/// the provided chain service adapter.
///
/// In case the epoch state for a given slot is not found, it will retry on
/// subsequent slots until one is successfully received.
pub struct EpochStream<SlotStream, ChainService, RuntimeServiceId>
where
    ChainService: CryptarchiaServiceData,
{
    slot_stream: SlotStream,
    chain_service_api: CryptarchiaServiceApi<ChainService, RuntimeServiceId>,
    current_epoch: Option<Epoch>,
    epoch_state_future: Option<GetEpochForSlotFuture>,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<SlotStream, ChainService, RuntimeServiceId>
    EpochStream<SlotStream, ChainService, RuntimeServiceId>
where
    ChainService: CryptarchiaServiceData,
{
    pub fn new(
        slot_stream: SlotStream,
        chain_service_api: CryptarchiaServiceApi<ChainService, RuntimeServiceId>,
    ) -> Self {
        Self {
            slot_stream,
            chain_service_api,
            current_epoch: None,
            epoch_state_future: None,
            _phantom: PhantomData,
        }
    }
}

#[expect(
    clippy::cognitive_complexity,
    reason = "It's better to keep all the polling logic together."
)]
impl<SlotStream, ChainService, RuntimeServiceId> Stream
    for EpochStream<SlotStream, ChainService, RuntimeServiceId>
where
    SlotStream: Stream<Item = SlotTick> + Unpin,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync>,
    RuntimeServiceId: Send + Sync + 'static + Unpin,
{
    type Item = EpochInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        tracing::debug!(target: LOG_TARGET, "Polling epoch stream for next epoch info...");
        let this = self.as_mut().get_mut();

        if let Some(ref mut pending_fut) = this.epoch_state_future {
            tracing::trace!(target: LOG_TARGET, "Pending future found to fetch state for current epoch...");
            match pending_fut.poll_unpin(cx) {
                Poll::Pending => {
                    tracing::trace!(target: LOG_TARGET, "Future not completed yet.");
                    return Poll::Pending;
                }
                Poll::Ready(epoch_state) => {
                    tracing::debug!(target: LOG_TARGET, "Future ready with result: {epoch_state:?}.");
                    this.epoch_state_future = None;
                    let Some(epoch_state) = epoch_state else {
                        // If we could not fetch the epoch state for the given slot, we reset
                        // `current_epoch` so we try again on the next slot tick.
                        tracing::warn!(target: LOG_TARGET, "No epoch state for given slot. Retrying on the next slot tick.");
                        this.current_epoch = None;
                        // We wake so we poll for the next slot tick.
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    };
                    return Poll::Ready(Some(epoch_state.into()));
                }
            }
        }
        tracing::debug!(target: LOG_TARGET, "Polling slot stream \u{1f555}...");
        match this.slot_stream.poll_next_unpin(cx) {
            Poll::Ready(None) => {
                tracing::debug!(target: LOG_TARGET, "Slot stream completed with `None`.");
                Poll::Ready(None)
            }
            Poll::Pending => {
                tracing::trace!(target: LOG_TARGET, "Slot stream not ready yet.");
                Poll::Pending
            }
            Poll::Ready(Some(SlotTick { epoch, slot })) => {
                // Same epoch as the already processed one, we don't yield anything new.
                if let Some(current_epoch) = this.current_epoch
                    && current_epoch == epoch
                {
                    tracing::debug!(target: LOG_TARGET, "New slot for current epoch. Skipping...");
                    // We wake so we poll for the next slot tick.
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                this.current_epoch = Some(epoch);
                let chain_service_clone = this.chain_service_api.clone();
                self.epoch_state_future = Some(Box::pin(async move {
                    chain_service_clone.get_epoch_state(slot).await.unwrap()
                }));
                tracing::debug!(target: LOG_TARGET, "Found new epoch unseen before. Polling for its state...");
                // We wake so we poll for the next slot tick.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use core::task::{Context, Poll};

//     use async_trait::async_trait;
//     use cryptarchia_engine::Slot;
//     use futures::{
//         StreamExt as _,
//         future::ready,
//         stream::{empty, once},
//         task::noop_waker_ref,
//     };
//     use nomos_core::crypto::ZkHash;
//     use nomos_ledger::{EpochState, UtxoTree};
//     use nomos_time::SlotTick;
//     use overwatch::overwatch::OverwatchHandle;
//     use test_log::test;

//     use crate::epoch::EpochStream;

//     const NON_EXISTING_EPOCH_STATE_SLOT: Slot = Slot::new(4);

//     fn default_epoch_state() -> EpochState {
//         use groth16::Field as _;

//         EpochState {
//             epoch: 1.into(),
//             nonce: ZkHash::ZERO,
//             total_stake: 1_000,
//             utxos: UtxoTree::new(),
//         }
//     }

//     #[derive(Clone)]
//     struct ChainService;

//     #[test(tokio::test)]
//     async fn manual_polling() {
//         let slot_stream = once(ready(SlotTick {
//             epoch: 1.into(),
//             slot: 1.into(),
//         }))
//         // New slot same epoch
//         .chain(once(ready(SlotTick {
//             epoch: 1.into(),
//             slot: 2.into(),
//         })))
//         // New slot new epoch
//         .chain(once(ready(SlotTick {
//             epoch: 2.into(),
//             slot: 3.into(),
//         })))
//         // New slot new epoch, but no associated epoch state
//         .chain(once(ready(SlotTick {
//             epoch: 3.into(),
//             slot: NON_EXISTING_EPOCH_STATE_SLOT,
//         })))
//         // New slot same epoch, but with associated epoch state
//         .chain(once(ready(SlotTick {
//             epoch: 3.into(),
//             slot: 5.into(),
//         })));
//         let mut stream = EpochStream::new(slot_stream, ChainService);

//         let mut cx = Context::from_waker(noop_waker_ref());
//         // First poll of the stream will set the epoch info future to fetch
// the         // information.
//         let poll_next = stream.poll_next_unpin(&mut cx);
//         assert_eq!(poll_next, Poll::Pending);
//         assert!(stream.epoch_state_future.is_some());
//         assert_eq!(stream.current_epoch, Some(1.into()));

//         // We wait until the inner future completes
//         let result = stream.next().await;
//         assert_eq!(result, Some(default_epoch_state().into()));
//         assert!(stream.epoch_state_future.is_none());
//         assert_eq!(stream.current_epoch, Some(1.into()));

//         // A new slot for the same epoch is used here, so nothing changes.
//         let poll_next = stream.poll_next_unpin(&mut cx);
//         assert_eq!(poll_next, Poll::Pending);
//         assert!(stream.epoch_state_future.is_none());
//         assert_eq!(stream.current_epoch, Some(1.into()));

//         // A new epoch is returned here, so the new future is set up and
// polled.         let poll_next = stream.poll_next_unpin(&mut cx);
//         assert_eq!(poll_next, Poll::Pending);
//         assert!(stream.epoch_state_future.is_some());
//         assert_eq!(stream.current_epoch, Some(2.into()));

//         // We wait until the inner future completes
//         let result = stream.next().await;
//         assert_eq!(result, Some(default_epoch_state().into()));
//         assert!(stream.epoch_state_future.is_none());
//         assert_eq!(stream.current_epoch, Some(2.into()));

//         // Next poll for new epoch, we spawn the future.
//         let poll_next = stream.poll_next_unpin(&mut cx);
//         assert_eq!(poll_next, Poll::Pending);
//         assert!(stream.epoch_state_future.is_some());
//         assert_eq!(stream.current_epoch, Some(3.into()));

//         // Once the future completes, it will return `None`, so the next slot
// will be         // polled.
//         loop {
//             let poll_next = stream.poll_next_unpin(&mut cx);
//             // React to the fact that `None` was received by the future.
//             if stream.current_epoch.is_none() {
//                 assert_eq!(poll_next, Poll::Pending);
//                 assert!(stream.epoch_state_future.is_none());
//                 break;
//             }
//         }

//         // Lastly, a new slot for the same epoch as the previous `None` is
// received, and         // the expected state is yielded.
//         let result = stream.next().await;
//         assert_eq!(result, Some(default_epoch_state().into()));
//         assert!(stream.epoch_state_future.is_none());
//         assert_eq!(stream.current_epoch, Some(3.into()));
//     }

//     #[test(tokio::test)]
//     async fn stream_api() {
//         let slot_stream = once(ready(SlotTick {
//             epoch: 1.into(),
//             slot: 1.into(),
//         }))
//         // New slot same epoch
//         .chain(once(ready(SlotTick {
//             epoch: 1.into(),
//             slot: 2.into(),
//         })))
//         // New slot new epoch
//         .chain(once(ready(SlotTick {
//             epoch: 2.into(),
//             slot: 3.into(),
//         })))
//         // New slot new epoch, but no associated epoch state
//         .chain(once(ready(SlotTick {
//             epoch: 3.into(),
//             slot: NON_EXISTING_EPOCH_STATE_SLOT,
//         })))
//         // New slot new epoch, but with associated epoch state
//         .chain(once(ready(SlotTick {
//             epoch: 4.into(),
//             slot: 5.into(),
//         })))
//         // We stop the iteration here
//         .chain(empty());
//         let mut stream = EpochStream::new(slot_stream, ChainService);

//         // Epoch 1
//         let next = stream.next().await;
//         assert_eq!(next, Some(default_epoch_state().into()));
//         assert_eq!(stream.current_epoch, Some(1.into()));
//         assert!(stream.epoch_state_future.is_none());

//         // Epoch 2
//         let next = stream.next().await;
//         assert_eq!(next, Some(default_epoch_state().into()));
//         assert_eq!(stream.current_epoch, Some(2.into()));
//         assert!(stream.epoch_state_future.is_none());

//         // Epoch 4 (skipping epoch 3 because no epoch state was found for it)
//         let next = stream.next().await;
//         assert_eq!(next, Some(default_epoch_state().into()));
//         assert_eq!(stream.current_epoch, Some(4.into()));
//         assert!(stream.epoch_state_future.is_none());

//         // Epoch 5 - should return `Ready(None)`
//         let next = stream.next().await;
//         assert!(next.is_none());
//     }
// }
