use core::{fmt::Debug, hash::Hash};
use std::fmt::Display;

use nomos_core::header::HeaderId;
use nomos_network::backends::NetworkBackend;
use overwatch::{DynError, services::AsServiceId};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tx_service::{MempoolMsg, TxMempoolService, backend::Mempool, network::NetworkAdapter};

pub async fn add_tx<
    MempoolNetworkBackend,
    MempoolNetworkAdapter,
    StorageAdapter,
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
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<RuntimeServiceId, Key = Key, Item = Item>
        + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    Item: Clone + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    Key: Clone + Debug + Ord + Hash + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + AsServiceId<
            TxMempoolService<
                MempoolNetworkAdapter,
                Mempool<HeaderId, Item, Key, StorageAdapter, RuntimeServiceId>,
                StorageAdapter,
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
