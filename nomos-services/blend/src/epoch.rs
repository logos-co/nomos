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
    services::{AsServiceId, ServiceCore, relay::RelayError},
};
use thiserror::Error;
use tokio::sync::oneshot::{self, error::RecvError};

pub struct EpochInfo {
    nonce: ZkHash,
    ledger_aged: ZkHash,
    total_stake: u64,
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

pub struct GetEpochStateForSlotMessage {
    pub slot: Slot,
    pub sender: oneshot::Sender<Option<EpochState>>,
}

impl<SlotStream, ChainService, RuntimeServiceId> Stream
    for EpochStream<SlotStream, ChainService, RuntimeServiceId>
where
    SlotStream: Stream<Item = SlotTick> + Unpin,
    ChainService: ServiceCore<RuntimeServiceId, Message: From<GetEpochStateForSlotMessage> + Send>
        + Unpin
        + 'static,
    RuntimeServiceId: Debug + Send + Sync + Clone + Display + AsServiceId<ChainService> + 'static,
{
    type Item = EpochInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(mut pending_fut) = self.epoch_state_future.take() {
            match pending_fut.poll_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(epoch_info)) => {
                    return Poll::Ready(epoch_info);
                }
                Poll::Ready(Err(_)) => panic!("Should not panic"),
            }
        }
        match self.slot_stream.poll_next_unpin(cx) {
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(SlotTick { epoch, slot })) => {
                // Same epoch as the already processed one, we don't yield anything new.
                if let Some(current_epoch) = self.current_epoch
                    && current_epoch == epoch
                {
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
    ChainService:
        ServiceCore<RuntimeServiceId, Message: From<GetEpochStateForSlotMessage> + Send + 'static>,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ChainService>,
{
    let relay = overwatch_handle
        .relay::<ChainService>()
        .await
        .map_err(Error::Relay)?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(GetEpochStateForSlotMessage { slot, sender }.into())
        .await
        .map_err(|_| Error::MsgSend)?;

    let epoch_state = receiver.await.map_err(Error::MsgRecv)?;
    let Some(EpochState {
        nonce,
        total_stake,
        utxos,
        ..
    }) = epoch_state
    else {
        return Ok(None);
    };
    Ok(Some(EpochInfo {
        ledger_aged: utxos.root(),
        nonce,
        total_stake,
    }))
}
