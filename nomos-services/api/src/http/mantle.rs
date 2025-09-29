use core::fmt::Debug;
use std::fmt::Display;

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService, BlockInfo};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, Transaction},
};
use nomos_mempool::{
    MempoolMetrics, MempoolMsg, TxMempoolService, backend::mockpool::MockPool,
    network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
    tx::service::openapi::Status,
};
use overwatch::services::AsServiceId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;

pub type MempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId> =
    TxMempoolService<
        MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
        SamplingNetworkAdapter,
        SamplingStorage,
        MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
        RuntimeServiceId,
    >;

pub async fn mantle_mempool_metrics<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<MempoolMetrics, super::DynError>
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<MempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
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

pub async fn mantle_mempool_status<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    items: Vec<<Tx as Transaction>::Hash>,
) -> Result<Vec<Status<HeaderId>>, super::DynError>
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<MempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
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
