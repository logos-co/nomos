use core::fmt::Debug;
use std::fmt::Display;

use nomos_core::{
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, Transaction},
};
use nomos_mempool::{
    backend::mockpool::MockPool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
    tx::service::openapi::Status, MempoolMetrics, MempoolMsg, TxMempoolService,
};
use overwatch::services::AsServiceId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::wait_with_timeout;

pub type ClMempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId> =
    TxMempoolService<
        MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
        SamplingNetworkAdapter,
        SamplingStorage,
        MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
        RuntimeServiceId,
    >;

pub async fn cl_mempool_metrics<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
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
        + AsServiceId<ClMempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(MempoolMsg::Metrics {
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for cl_mempool_metrics".to_owned(),
    )
    .await
}

pub async fn cl_mempool_status<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
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
        + AsServiceId<ClMempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
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

    wait_with_timeout(
        receiver,
        "Timeout while waiting for cl_mempool_status".to_owned(),
    )
    .await
}
