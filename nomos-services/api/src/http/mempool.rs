use core::{fmt::Debug, hash::Hash};
use std::fmt::Display;

use nomos_core::header::HeaderId;
use nomos_da_sampling::network::NetworkAdapter as DaSamplingNetworkAdapter;
use nomos_mempool::{
    backend::mockpool::MockPool, network::NetworkAdapter, MempoolMsg, TxMempoolService,
};
use nomos_network::backends::NetworkBackend;
use overwatch::{services::AsServiceId, DynError};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub async fn add_tx<
    MempoolNetworkBackend,
    MempoolNetworkAdapter,
    SamplingNetworkAdapter,
    SamplingStorage,
    Item,
    Key,
    RuntimeServiceId,
>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    item: Item,
    converter: impl Fn(&Item) -> Key,
) -> Result<(), DynError>
where
    MempoolNetworkBackend: NetworkBackend<RuntimeServiceId>,
    MempoolNetworkAdapter: NetworkAdapter<RuntimeServiceId, Backend = MempoolNetworkBackend, Payload = Item, Key = Key>
        + Send
        + Sync
        + 'static,
    MempoolNetworkAdapter::Settings: Send + Sync,
    SamplingNetworkAdapter: DaSamplingNetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    Item: Clone + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    Key: Clone + Debug + Ord + Hash + Send + Serialize + for<'de> Deserialize<'de> + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<
            TxMempoolService<
                MempoolNetworkAdapter,
                SamplingNetworkAdapter,
                SamplingStorage,
                MockPool<HeaderId, Item, Key>,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(MempoolMsg::Add {
            key: converter(&item),
            payload: item,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    receiver
        .await
        .map_err(|_| DynError::from("Failed to add tx"))?
        .map_err(DynError::from)
}
