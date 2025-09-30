use core::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use async_trait::async_trait;
use cryptarchia_engine::{Epoch, Slot};
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

#[async_trait]
pub trait ChainApi {
    async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState>;
}

pub struct EpochStream<SlotStream, ChainService> {
    slot_stream: SlotStream,
    chain_service: ChainService,
    current_epoch: Option<Epoch>,
    epoch_state_future: Option<GetEpochForSlotFuture>,
}

impl<SlotStream, ChainService> EpochStream<SlotStream, ChainService> {
    pub fn new(slot_stream: SlotStream, chain_service: ChainService) -> Self {
        Self {
            slot_stream,
            chain_service,
            current_epoch: None,
            epoch_state_future: None,
        }
    }
}

impl<SlotStream, ChainService> Stream for EpochStream<SlotStream, ChainService>
where
    SlotStream: Stream<Item = SlotTick> + Unpin,
    ChainService: ChainApi + Clone + Unpin + Send + 'static,
{
    type Item = EpochInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        tracing::debug!(target: LOG_TARGET, "Polling epoch stream for next epoch info...");
        let this = self.as_mut().get_mut();

        if let Some(ref mut pending_fut) = this.epoch_state_future {
            tracing::trace!(target: LOG_TARGET, "Future found to fetch state for current epoch...");
            match pending_fut.poll_unpin(cx) {
                Poll::Pending => {
                    tracing::trace!(target: LOG_TARGET, "Future not completed yet.");
                    return Poll::Pending;
                }
                Poll::Ready(epoch_state) => {
                    tracing::debug!(target: LOG_TARGET, "Future ready with result: {epoch_state:?}.");
                    this.epoch_state_future = None;
                    return Poll::Ready(epoch_state.map(Into::into));
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
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                this.current_epoch = Some(epoch);
                let chain_service_clone = this.chain_service.clone();
                self.epoch_state_future = Some(Box::pin(async move {
                    chain_service_clone.get_epoch_state_for_slot(slot).await
                }));
                tracing::debug!(target: LOG_TARGET, "Found new epoch unseen before. Polling for its state...");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::task::{Context, Poll};

    use async_trait::async_trait;
    use cryptarchia_engine::Slot;
    use futures::{StreamExt as _, future::ready, stream::once, task::noop_waker_ref};
    use nomos_core::crypto::ZkHash;
    use nomos_ledger::{EpochState, UtxoTree};
    use nomos_time::SlotTick;
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
    impl ChainApi for ChainService {
        async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState> {
            if slot == NON_EXISTING_EPOCH_STATE_SLOT {
                None
            } else {
                Some(default_epoch_state())
            }
        }
    }

    #[test(tokio::test)]
    async fn polling() {
        let slot_stream = once(ready(SlotTick {
            epoch: 1.into(),
            slot: 1.into(),
        }))
        // New slot same epoch
        .chain(once(ready(SlotTick {
            epoch: 1.into(),
            slot: 2.into(),
        })))
        // New slot new epoch
        .chain(once(ready(SlotTick {
            epoch: 2.into(),
            slot: 3.into(),
        })))
        // New slot same epoch, but no associated epoch state
        .chain(once(ready(SlotTick {
            epoch: 2.into(),
            slot: NON_EXISTING_EPOCH_STATE_SLOT,
        })));
        let mut stream = EpochStream::new(slot_stream, ChainService);

        let mut cx = Context::from_waker(noop_waker_ref());
        // First poll of the stream will set the epoch info future to fetch the
        // information.
        let poll_next = stream.poll_next_unpin(&mut cx);
        assert_eq!(poll_next, Poll::Pending);
        assert!(stream.epoch_state_future.is_some());
        assert_eq!(stream.current_epoch, Some(1.into()));

        // We wait until the inner future completes
        let result = stream.next().await;
        assert_eq!(result, Some(default_epoch_state().into()));
        assert!(stream.epoch_state_future.is_none());
        assert_eq!(stream.current_epoch, Some(1.into()));

        // A new slot for the same epoch is used here, so nothing changes.
        let poll_next = stream.poll_next_unpin(&mut cx);
        assert_eq!(poll_next, Poll::Pending);
        assert!(stream.epoch_state_future.is_none());
        assert_eq!(stream.current_epoch, Some(1.into()));

        // A new epoch is returned here, so the new future is set up and polled.
        let poll_next = stream.poll_next_unpin(&mut cx);
        assert_eq!(poll_next, Poll::Pending);
        assert!(stream.epoch_state_future.is_some());
        assert_eq!(stream.current_epoch, Some(2.into()));

        // We wait until the inner future completes
        let result = stream.next().await;
        assert_eq!(result, Some(default_epoch_state().into()));
        assert!(stream.epoch_state_future.is_none());
        assert_eq!(stream.current_epoch, Some(2.into()));

        // Next slot should return a `None` epoch state
        let result = stream.next().await;
        assert!(result.is_none());
    }
}
