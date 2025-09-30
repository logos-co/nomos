use core::{
    fmt::{Debug, Display},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use chain_service::{ConsensusMsg, CryptarchiaConsensus};
use cryptarchia_engine::{Epoch, Slot};
use futures::{FutureExt, Stream, StreamExt as _, pin_mut};
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

pub struct EpochStream<SlotStream, ChainService, Tx, RuntimeServiceId> {
    slot_stream: SlotStream,
    current_epoch: Option<Epoch>,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    _phantom: PhantomData<(ChainService, Tx)>,
}

impl<SlotStream, ChainService, Tx, RuntimeServiceId>
    EpochStream<SlotStream, ChainService, Tx, RuntimeServiceId>
{
    pub fn new(
        slot_stream: SlotStream,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Self {
        Self {
            current_epoch: None,
            overwatch_handle,
            slot_stream,
            _phantom: PhantomData,
        }
    }
}

impl<SlotStream, ChainService, Tx, RuntimeServiceId> Stream
    for EpochStream<SlotStream, ChainService, Tx, RuntimeServiceId>
where
    SlotStream: Stream<Item = SlotTick> + Unpin,
    ChainService: ServiceCore<RuntimeServiceId, Message = ConsensusMsg<Tx>> + Unpin,
    Tx: Send + 'static + Unpin,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ChainService>,
{
    type Item = EpochInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.slot_stream.poll_next_unpin(cx) {
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(SlotTick { epoch, slot })) => {
                // Same epoch as the already processed one, we don't yield anyting new.
                if let Some(current_epoch) = self.current_epoch
                    && current_epoch == epoch
                {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                };
                self.current_epoch = Some(epoch);
                let mut epoch_for_slot_future =
                    get_epoch_state_for_slot(slot, &self.overwatch_handle);
                // TODO: Must resume from here
                match pin_mut!(epoch_for_slot_future).poll_unpin(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(epoch_info)) => return Poll::Ready(epoch_info),
                    _ => panic!("Should not panic"),
                }
            }
        }
    }
}

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    RelayError(RelayError),
    #[error("Failed to send message to get epoch state.")]
    MsgSendError,
    #[error(transparent)]
    MsgRecvError(RecvError),
}

async fn get_epoch_state_for_slot<ChainService, Tx, RuntimeServiceId>(
    slot: Slot,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<Option<EpochInfo>, Error>
where
    ChainService: ServiceCore<RuntimeServiceId, Message = ConsensusMsg<Tx>>,
    Tx: Send + 'static,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ChainService>,
{
    let relay = overwatch_handle
        .relay::<ChainService>()
        .await
        .map_err(Error::RelayError)?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(ConsensusMsg::GetEpochState { slot, tx: sender })
        .await
        .map_err(|_| Error::MsgSendError)?;

    let epoch_state = receiver.await.map_err(Error::MsgRecvError)?;
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
