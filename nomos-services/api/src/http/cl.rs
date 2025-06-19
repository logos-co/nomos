use core::fmt::Debug;
use std::fmt::Display;

use nomos_core::{header::HeaderId, mantle::Transaction};
use nomos_mempool::{
    backend::mockpool::MockPool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
    tx::service::openapi::Status, MempoolMetrics, MempoolMsg, TxMempoolService,
};
use overwatch::services::AsServiceId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::wait_with_timeout;

pub type ClMempoolService<T, RuntimeServiceId> = TxMempoolService<
    MempoolNetworkAdapter<T, <T as Transaction>::Hash, RuntimeServiceId>,
    MockPool<HeaderId, T, <T as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub async fn cl_mempool_metrics<T, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<MempoolMetrics, super::DynError>
where
    T: Transaction + Clone + Debug + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
    <T as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ClMempoolService<T, RuntimeServiceId>>,
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

pub async fn cl_mempool_status<T, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    items: Vec<<T as Transaction>::Hash>,
) -> Result<Vec<Status<HeaderId>>, super::DynError>
where
    T: Transaction + Clone + Debug + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
    <T as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    RuntimeServiceId: Debug + Sync + Display + AsServiceId<ClMempoolService<T, RuntimeServiceId>>,
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
