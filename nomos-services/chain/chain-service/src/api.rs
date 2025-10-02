use nomos_core::{block::Block, header::HeaderId};
use overwatch::{
    DynError,
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use tokio::sync::{broadcast, oneshot};

use crate::{ConsensusMsg, CryptarchiaInfo, LibUpdate};

pub trait CryptarchiaServiceData:
    ServiceData<Message = ConsensusMsg<Self::Tx>> + Send + 'static
{
    type Tx;
}
impl<T, Tx> CryptarchiaServiceData for T
where
    T: ServiceData<Message = ConsensusMsg<Tx>> + Send + 'static,
{
    type Tx = Tx;
}

pub struct CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
{
    relay: OutboundRelay<Cryptarchia::Message>,
    _id: std::marker::PhantomData<RuntimeServiceId>,
}

impl<Cryptarchia, RuntimeServiceId> Clone for CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
{
    fn clone(&self) -> Self {
        Self {
            relay: self.relay.clone(),
            _id: std::marker::PhantomData,
        }
    }
}

impl<Cryptarchia, RuntimeServiceId> CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
    RuntimeServiceId: AsServiceId<Cryptarchia> + std::fmt::Debug + std::fmt::Display + Sync,
{
    /// Create a new API instance
    pub async fn new(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Result<Self, DynError> {
        let relay = overwatch_handle.relay::<Cryptarchia>().await?;

        Ok(Self {
            relay,
            _id: std::marker::PhantomData,
        })
    }
}

impl<Cryptarchia, RuntimeServiceId> CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    RuntimeServiceId: Sync,
{
    /// Get the current consensus info including LIB, tip, slot, height, and
    /// mode
    pub async fn info(&self) -> Result<CryptarchiaInfo, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::Info { tx })
            .await
            .map_err(|_| "Failed to send info request")?;

        Ok(rx.await?)
    }

    /// Subscribe to new blocks
    pub async fn subscribe_new_blocks(&self) -> Result<broadcast::Receiver<HeaderId>, DynError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::NewBlockSubscribe { sender })
            .await
            .map_err(|_| "Failed to send block subscription request")?;

        Ok(receiver.await?)
    }

    /// Subscribe to LIB (Last Immutable Block) updates
    pub async fn subscribe_lib_updates(&self) -> Result<broadcast::Receiver<LibUpdate>, DynError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::LibSubscribe { sender })
            .await
            .map_err(|_| "Failed to send LIB subscription request")?;

        Ok(receiver.await?)
    }

    /// Get headers in the range from `from` to `to`
    /// If `from` is None, defaults to tip
    /// If `to` is None, defaults to LIB
    pub async fn get_headers(
        &self,
        from: Option<HeaderId>,
        to: Option<HeaderId>,
    ) -> Result<Vec<HeaderId>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::GetHeaders { from, to, tx })
            .await
            .map_err(|_| "Failed to send headers request")?;

        Ok(rx.await?)
    }

    /// Get all headers from a specific block to LIB
    pub async fn get_headers_to_lib(&self, from: HeaderId) -> Result<Vec<HeaderId>, DynError> {
        self.get_headers(Some(from), None).await
    }

    /// Get all headers from tip to a specific block
    pub async fn get_headers_from_tip(&self, to: HeaderId) -> Result<Vec<HeaderId>, DynError> {
        self.get_headers(None, Some(to)).await
    }

    /// Get the ledger state at a specific block
    pub async fn get_ledger_state(
        &self,
        block_id: HeaderId,
    ) -> Result<Option<nomos_ledger::LedgerState>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::GetLedgerState { block_id, tx })
            .await
            .map_err(|_| "Failed to send ledger state request")?;

        Ok(rx.await?)
    }

    /// Get the epoch state for a given slot
    pub async fn get_epoch_state(
        &self,
        slot: cryptarchia_engine::Slot,
    ) -> Result<Option<nomos_ledger::EpochState>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::GetEpochState { slot, tx })
            .await
            .map_err(|_| "Failed to send epoch state request")?;

        Ok(rx.await?)
    }

    /// Process a block through the chain service
    pub async fn process_leader_block(
        &self,
        block: Block<Cryptarchia::Tx>,
    ) -> Result<(), DynError> {
        let (tx, rx) = oneshot::channel();

        let boxed_block = Box::new(block);
        self.relay
            .send(ConsensusMsg::ProcessLeaderBlock {
                block: boxed_block,
                tx,
            })
            .await
            .map_err(|_| "Failed to send process block request")?;

        rx.await?.map_err(Into::into)
    }
}
