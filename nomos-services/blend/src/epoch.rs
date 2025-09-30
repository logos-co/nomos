use core::{
    fmt::{Debug, Display},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use cryptarchia_engine::{Epoch, Slot};
use futures::{FutureExt as _, Stream, StreamExt as _};
use nomos_core::crypto::ZkHash;
use nomos_ledger::EpochState;
use nomos_time::SlotTick;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData, relay::RelayError},
};
use thiserror::Error;
use tokio::sync::oneshot::{Sender, channel, error::RecvError};

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

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Relay(RelayError),
    #[error("Failed to send message to get epoch state.")]
    MsgSend,
    #[error(transparent)]
    MsgRecv(RecvError),
}

type GetEpochForSlotFuture = Pin<Box<dyn Future<Output = Result<Option<EpochInfo>, Error>> + Send>>;

pub struct EpochStream<SlotStream, ChainService, RuntimeServiceId> {
    slot_stream: SlotStream,
    current_epoch: Option<Epoch>,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    epoch_state_future: Option<GetEpochForSlotFuture>,
    _phantom: PhantomData<ChainService>,
}

impl<SlotStream, ChainService, RuntimeServiceId>
    EpochStream<SlotStream, ChainService, RuntimeServiceId>
{
    pub fn new(
        slot_stream: SlotStream,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Self {
        Self {
            current_epoch: None,
            overwatch_handle,
            slot_stream,
            epoch_state_future: None,
            _phantom: PhantomData,
        }
    }
}

#[derive(Debug)]
pub struct GetEpochStateForSlotMessage {
    pub slot: Slot,
    pub sender: Sender<Option<EpochState>>,
}

impl<SlotStream, ChainService, RuntimeServiceId> Stream
    for EpochStream<SlotStream, ChainService, RuntimeServiceId>
where
    SlotStream: Stream<Item = SlotTick> + Unpin,
    ChainService: ServiceData<Message: From<GetEpochStateForSlotMessage> + Send> + Unpin + 'static,
    RuntimeServiceId: Debug + Send + Sync + Clone + Display + AsServiceId<ChainService> + 'static,
{
    type Item = EpochInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        tracing::debug!(target: LOG_TARGET, "Polling epoch stream for next epoch info...");
        if let Some(ref mut pending_fut) = self.epoch_state_future {
            tracing::trace!(target: LOG_TARGET, "Future found to fetch state for current epoch...");
            match pending_fut.poll_unpin(cx) {
                Poll::Pending => {
                    tracing::trace!(target: LOG_TARGET, "Future not completed yet.");
                    return Poll::Pending;
                }
                Poll::Ready(Ok(epoch_info)) => {
                    tracing::debug!(target: LOG_TARGET, "Future ready with result: {epoch_info:?}.");
                    self.epoch_state_future = None;
                    return Poll::Ready(epoch_info);
                }
                Poll::Ready(Err(_)) => {
                    panic!("Epoch state for slot returned by time service should always exist.")
                }
            }
        }
        tracing::debug!(target: LOG_TARGET, "Polling slot stream \u{1f555}...");
        match self.slot_stream.poll_next_unpin(cx) {
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
                if let Some(current_epoch) = self.current_epoch
                    && current_epoch == epoch
                {
                    tracing::debug!(target: LOG_TARGET, "New slot for current epoch. Skipping...");
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                self.current_epoch = Some(epoch);
                self.epoch_state_future = Some(Box::pin(get_epoch_state_for_slot::<
                    ChainService,
                    RuntimeServiceId,
                >(
                    slot, self.overwatch_handle.clone()
                )));
                tracing::debug!(target: LOG_TARGET, "Found new epoch unseen before. Polling for its state...");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

async fn get_epoch_state_for_slot<ChainService, RuntimeServiceId>(
    slot: Slot,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
) -> Result<Option<EpochInfo>, Error>
where
    ChainService: ServiceData<Message: From<GetEpochStateForSlotMessage> + Send + 'static>,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ChainService>,
{
    let relay = overwatch_handle
        .relay::<ChainService>()
        .await
        .map_err(Error::Relay)?;
    let (sender, receiver) = channel();

    relay
        .send(GetEpochStateForSlotMessage { slot, sender }.into())
        .await
        .map_err(|_| Error::MsgSend)?;

    let epoch_state = receiver.await.map_err(Error::MsgRecv)?;
    Ok(epoch_state.map(Into::into))
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
    use overwatch::{
        DynError, OpaqueServiceResourcesHandle, derive_services,
        overwatch::OverwatchRunner,
        services::{
            ServiceCore, ServiceData,
            state::{NoOperator, NoState},
        },
    };
    use test_log::test;

    use crate::epoch::{EpochStream, GetEpochStateForSlotMessage};

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

    struct MockService<RuntimeServiceId>(OpaqueServiceResourcesHandle<Self, RuntimeServiceId>);

    impl<RuntimeServiceId> ServiceData for MockService<RuntimeServiceId> {
        const SERVICE_RELAY_BUFFER_SIZE: usize = 1;
        type Message = GetEpochStateForSlotMessage;
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
    }

    #[async_trait]
    impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for MockService<RuntimeServiceId>
    where
        RuntimeServiceId: Send,
    {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self(service_resources_handle))
        }

        async fn run(mut self) -> Result<(), DynError> {
            while let Some(GetEpochStateForSlotMessage { sender, slot }) =
                self.0.inbound_relay.next().await
            {
                let epoch_state = (slot != NON_EXISTING_EPOCH_STATE_SLOT).then(default_epoch_state);
                sender.send(epoch_state).unwrap();
            }
            Ok(())
        }
    }

    #[derive_services]
    struct Test {
        mock: MockService<RuntimeServiceId>,
    }

    #[test(test)]
    fn polling() {
        let overwatch =
            OverwatchRunner::<Test>::run(TestServiceSettings { mock: () }, None).unwrap();
        let handle = overwatch.handle().clone();
        handle
            .runtime()
            .block_on(handle.start_service::<MockService<RuntimeServiceId>>())
            .expect("service to start successfully.");

        let handle_clone = overwatch.handle().clone();
        handle.runtime().block_on(async move {
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
            let mut stream =
                EpochStream::<_, MockService<RuntimeServiceId>, _>::new(slot_stream, handle_clone);

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
        });

        handle.runtime().block_on(handle.shutdown()).unwrap();
    }
}
