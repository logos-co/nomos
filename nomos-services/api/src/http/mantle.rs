use core::fmt::Debug;
use std::fmt::Display;

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService, BlockInfo};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction},
};
use overwatch::services::AsServiceId;
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tx_service::{
    MempoolMetrics, MempoolMsg, TxMempoolService, backend::Mempool,
    network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
    tx::service::openapi::Status,
};

pub type MempoolService<SamplingNetworkAdapter, SamplingStorage, StorageAdapter, RuntimeServiceId> =
    TxMempoolService<
        MempoolNetworkAdapter<
            SignedMantleTx,
            <SignedMantleTx as Transaction>::Hash,
            RuntimeServiceId,
        >,
        SamplingNetworkAdapter,
        SamplingStorage,
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

pub async fn mantle_mempool_metrics<
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    RuntimeServiceId,
>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<MempoolMetrics, super::DynError>
where
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
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
        + AsServiceId<
            MempoolService<
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                RuntimeServiceId,
            >,
        >,
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

pub async fn mantle_mempool_status<
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    RuntimeServiceId,
>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    items: Vec<<SignedMantleTx as Transaction>::Hash>,
) -> Result<Vec<Status<HeaderId>>, super::DynError>
where
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
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
        + AsServiceId<
            MempoolService<
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                RuntimeServiceId,
            >,
        >,
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
