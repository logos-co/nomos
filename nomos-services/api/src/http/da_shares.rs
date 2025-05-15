use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
    io::ErrorKind,
};

use bytes::Bytes;
use futures::{stream, Stream, StreamExt as _};
use nomos_core::da::blob::{LightShare, Share};
use nomos_storage::{
    api::da::StorageDaApi,
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{relay::OutboundRelay, AsServiceId},
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::oneshot;

pub(crate) type DaStorageBackend<SerdeOp> = RocksBackend<SerdeOp>;

#[expect(clippy::implicit_hasher, reason = "Don't need custom hasher")]
pub async fn get_shares<StorageOp, DaShare, RuntimeServiceId>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    blob_id: DaShare::BlobId,
    requested_shares: HashSet<DaShare::ShareIndex>,
    filter_shares: HashSet<DaShare::ShareIndex>,
    return_available: bool,
) -> Result<
    impl Stream<Item = Result<Bytes, crate::http::DynError>> + Send + Sync + 'static,
    crate::http::DynError,
>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share + 'static,
    DaShare::BlobId: AsRef<[u8]> + Clone + DeserializeOwned + Send + Sync + 'static,
    DaShare::ShareIndex: AsRef<[u8]> + DeserializeOwned + Hash + Eq + Send + Sync + 'static,
    DaShare::LightShare: LightShare<ShareIndex = DaShare::ShareIndex>
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    StorageOp::Error: std::error::Error + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<StorageService<RocksBackend<StorageOp>, RuntimeServiceId>>,
    // Service and storage layer types conversions
    <DaStorageBackend<StorageOp> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::ShareIndex:
        Into<DaShare::ShareIndex> + From<DaShare::ShareIndex>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::Share: TryInto<DaShare::LightShare>,
{
    let storage_relay = handle.relay().await?;

    let shares_indices =
        load_blob_shares_indices::<StorageOp, DaShare>(&storage_relay, blob_id.clone()).await?;

    let filtered_shares = shares_indices.into_iter().filter(move |idx| {
        // If requested_shares contains the index, then ignore the filter_shares
        requested_shares.contains(idx) || (return_available && !filter_shares.contains(idx))
    });

    let stream = stream::iter(filtered_shares).then(move |share_idx| {
        let storage_relay = storage_relay.clone();
        load_and_process_share::<StorageOp, DaShare>(storage_relay, blob_id.clone(), share_idx)
    });

    Ok(stream)
}

async fn load_blob_shares_indices<StorageOp, DaShare>(
    storage_relay: &OutboundRelay<StorageMsg<RocksBackend<StorageOp>>>,
    blob_id: DaShare::BlobId,
) -> Result<HashSet<DaShare::ShareIndex>, crate::http::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    DaShare::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    DaShare::ShareIndex: DeserializeOwned + Hash + Eq,
    StorageOp::Error: Send + Sync + 'static,
    // Service and storage layer types conversions
    <DaStorageBackend<StorageOp> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::ShareIndex: Into<DaShare::ShareIndex>,
{
    let (index_tx, index_rx) = oneshot::channel();
    storage_relay
        .send(StorageMsg::get_light_share_indexes_request(
            blob_id.into(),
            index_tx,
        ))
        .await
        .map_err(|(e, _)| Box::new(e) as crate::http::DynError)?;

    index_rx
        .await
        .map(|indexes| indexes.map(|data| data.into_iter().map(Into::into).collect::<HashSet<_>>()))
        .map_err(|e| Box::new(e) as crate::http::DynError)?
        .ok_or_else(|| {
            Box::new(std::io::Error::new(
                ErrorKind::NotFound,
                "Blob index not found",
            )) as crate::http::DynError
        })
}

async fn load_and_process_share<StorageOp, DaShare>(
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<StorageOp>>>,
    blob_id: <DaShare as Share>::BlobId,
    share_idx: DaShare::ShareIndex,
) -> Result<Bytes, crate::http::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    DaShare::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    DaShare::ShareIndex: AsRef<[u8]>,
    DaShare::LightShare: Serialize + DeserializeOwned,
    StorageOp::Error: std::error::Error + Send + Sync + 'static,
    // Service and storage layer types conversions
    <DaStorageBackend<StorageOp> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::ShareIndex:
        From<DaShare::ShareIndex> + Into<DaShare::ShareIndex>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::Share: TryInto<DaShare::LightShare>,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    storage_relay
        .send(StorageMsg::get_light_share_request(
            blob_id.into(),
            share_idx.into(),
            reply_tx,
        ))
        .await
        .map_err(|(e, _)| Box::new(e) as crate::http::DynError)?;

    let share = reply_rx
        .await
        .map_err(|e| Box::new(e) as crate::http::DynError)?
        .ok_or_else(|| {
            Box::new(std::io::Error::new(ErrorKind::NotFound, "Share not found"))
                as crate::http::DynError
        })?
        .try_into()
        .map_err(|_| {
            Box::new(std::io::Error::new(
                ErrorKind::InvalidData,
                "Failed to deserialize share",
            )) as crate::http::DynError
        })?;

    let mut json = serde_json::to_vec(&share).map_err(|e| Box::new(e) as crate::http::DynError)?;
    json.push(b'\n');

    Ok(Bytes::from(json))
}
