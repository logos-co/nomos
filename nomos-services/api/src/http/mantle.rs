use core::fmt::Debug;
use std::fmt::Display;

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService, BlockInfo};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction, ops::channel::ChannelId},
};
use nomos_ledger::mantle::channel::ChannelState;
use overwatch::services::AsServiceId;
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tx_service::{
    MempoolMetrics, MempoolMsg, TxMempoolService, backend::Mempool,
    network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
    tx::service::openapi::Status,
};

pub type MempoolService<StorageAdapter, RuntimeServiceId> = TxMempoolService<
    MempoolNetworkAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash, RuntimeServiceId>,
    Mempool<
        HeaderId,
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        StorageAdapter,
        RuntimeServiceId,
    >,
    StorageAdapter,
    RuntimeServiceId,
>;

pub async fn mantle_mempool_metrics<StorageAdapter, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<MempoolMetrics, super::DynError>
where
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Key = <SignedMantleTx as Transaction>::Hash,
            Item = SignedMantleTx,
        > + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<MempoolService<StorageAdapter, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(MempoolMsg::Metrics {
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    receiver.await.map_err(|e| Box::new(e) as super::DynError)
}

pub async fn mantle_mempool_status<StorageAdapter, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    items: Vec<<SignedMantleTx as Transaction>::Hash>,
) -> Result<Vec<Status<HeaderId>>, super::DynError>
where
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Key = <SignedMantleTx as Transaction>::Hash,
            Item = SignedMantleTx,
        > + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<MempoolService<StorageAdapter, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(MempoolMsg::Status {
            items,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    receiver.await.map_err(|e| Box::new(e) as super::DynError)
}

pub async fn lib_block_stream<RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<
    impl Stream<Item = Result<BlockInfo, crate::http::DynError>> + Send + Sync + use<RuntimeServiceId>,
    super::DynError,
>
where
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<BlockBroadcastService<RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(BlockBroadcastMsg::SubscribeToFinalizedBlocks {
            result_sender: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    let broadcast_receiver = receiver.await.map_err(|e| Box::new(e) as super::DynError)?;
    let stream = BroadcastStream::new(broadcast_receiver)
        .map(|result| result.map_err(|e| Box::new(e) as crate::http::DynError));

    Ok(stream)
}

pub async fn get_channel_state<
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    TimeBackend,
    RuntimeServiceId,
>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    channel_id: ChannelId,
) -> Result<Option<ChannelState>, super::DynError>
where
    SamplingBackend: nomos_da_sampling::backend::DaSamplingServiceBackend<BlobId = [u8; 32]> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Item = SignedMantleTx,
            Key = <SignedMantleTx as Transaction>::Hash,
        > + Send
        + Sync
        + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            super::consensus::Cryptarchia<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                TimeBackend,
                RuntimeServiceId,
            >,
        >,
{
    use chain_service::ConsensusMsg;

    let relay = handle
        .relay::<super::consensus::Cryptarchia<
            SamplingBackend,
            SamplingNetworkAdapter,
            SamplingStorage,
            StorageAdapter,
            TimeBackend,
            RuntimeServiceId,
        >>()
        .await?;
    let (info_sender, info_receiver) = oneshot::channel();

    relay
        .send(ConsensusMsg::Info { tx: info_sender })
        .await
        .map_err(|(e, _)| e)?;

    let info = info_receiver
        .await
        .map_err(|e| Box::new(e) as super::DynError)?;

    let (ledger_sender, ledger_receiver) = oneshot::channel();

    relay
        .send(ConsensusMsg::GetLedgerState {
            block_id: info.tip,
            tx: ledger_sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    let ledger_state = ledger_receiver
        .await
        .map_err(|e| Box::new(e) as super::DynError)?;

    Ok(ledger_state.and_then(|state| state.get_channel(&channel_id).cloned()))
}
