use core::{fmt::Debug, hash::Hash};
use std::fmt::Display;

use nomos_core::{da::blob::info::DispersedBlobInfo, header::HeaderId};
use nomos_da_sampling::{
    backend::DaSamplingServiceBackend, network::NetworkAdapter as DaSamplingNetworkAdapter,
};
use nomos_da_verifier::backend::VerifierBackend;
use nomos_mempool::{
    backend::mockpool::MockPool, network::NetworkAdapter, DaMempoolService, MempoolMsg,
    TxMempoolService,
};
use nomos_network::backends::NetworkBackend;
use overwatch::{services::AsServiceId, DynError};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::wait_with_timeout;

pub async fn add_tx<
    MempoolNetworkBackend,
    MempoolNetworkAdapter,
    SamplingNetworkAdapter,
    VerifierNetworkAdapter,
    SamplingStorage,
    VerifierStorage,
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
    VerifierNetworkAdapter:
        nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    VerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
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
                VerifierNetworkAdapter,
                SamplingStorage,
                VerifierStorage,
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

    wait_with_timeout(receiver, "Timeout while waiting for add_tx".to_owned())
        .await?
        .map_err(DynError::from)
}

pub async fn add_blob_info<
    N,
    A,
    Item,
    Key,
    SamplingBackend,
    SamplingAdapter,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    RuntimeServiceId,
>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    item: A::Payload,
    converter: impl Fn(&A::Payload) -> Key,
) -> Result<(), DynError>
where
    N: NetworkBackend<RuntimeServiceId>,
    A: NetworkAdapter<RuntimeServiceId, Backend = N, Key = Key> + Send + Sync + 'static,
    A::Payload: DispersedBlobInfo + Into<Item> + Debug,
    A::Settings: Send + Sync,
    Item: Clone + Debug + Send + Sync + 'static + Serialize + for<'de> Deserialize<'de>,
    Key: Clone + Debug + Ord + Hash + Send + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = Key> + Send,
    SamplingBackend::BlobId: Debug,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::Settings: Clone,
    SamplingAdapter: DaSamplingNetworkAdapter<RuntimeServiceId> + Send,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierBackend: VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaMempoolService<
                A,
                MockPool<HeaderId, Item, Key>,
                SamplingBackend,
                SamplingAdapter,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
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

    wait_with_timeout(
        receiver,
        "Timeout while waiting for add_blob_info".to_owned(),
    )
    .await?
    .map_err(DynError::from)
}
