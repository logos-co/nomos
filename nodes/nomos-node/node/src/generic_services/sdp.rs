use async_trait::async_trait;
use nomos_core::mantle::{SignedMantleTx, Transaction};
use nomos_sdp::adapters::mempool::SdpMempoolAdapter;
use overwatch::{
    DynError,
    overwatch::OverwatchHandle,
    services::{AsServiceId, relay::OutboundRelay},
};
use tokio::sync::oneshot;
use tx_service::MempoolMsg;

use crate::generic_services::TxMempoolService;

pub struct SdpTxPublisher<SamplingAdapter, RuntimeServiceId> {
    mempool_relay: OutboundRelay<
        MempoolMsg<
            nomos_core::header::HeaderId,
            SignedMantleTx,
            SignedMantleTx,
            <SignedMantleTx as Transaction>::Hash,
        >,
    >,
    _phantom: std::marker::PhantomData<(SamplingAdapter, RuntimeServiceId)>,
}

#[async_trait]
impl<SamplingAdapter, RuntimeServiceId> SdpMempoolAdapter<RuntimeServiceId>
    for SdpTxPublisher<SamplingAdapter, RuntimeServiceId>
where
    SamplingAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<TxMempoolService<SamplingAdapter, RuntimeServiceId>>
        + Send
        + Sync
        + 'static
        + std::fmt::Debug
        + std::fmt::Display,
{
    async fn new(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) -> Result<Self, DynError> {
        let mempool_relay = overwatch_handle
            .relay::<TxMempoolService<SamplingAdapter, RuntimeServiceId>>()
            .await?;

        Ok(Self {
            mempool_relay,
            _phantom: std::marker::PhantomData,
        })
    }

    async fn post_tx(&self, tx: SignedMantleTx) -> Result<(), DynError> {
        let (reply_channel, receiver) = oneshot::channel();
        let key = tx.mantle_tx.hash();

        self.mempool_relay
            .send(MempoolMsg::Add {
                payload: tx,
                key,
                reply_channel,
            })
            .await
            .map_err(|(e, _)| e.into())?;

        receiver.await?.map_err(|e| e.into())?;

        Ok(())
    }
}
