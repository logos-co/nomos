use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
    io::ErrorKind,
    sync::Arc,
};

use bytes::Bytes;
use futures::{stream, Stream, StreamExt as _};
use nomos_core::da::blob::{LightShare, Share};
use nomos_da_storage::rocksdb::{
    create_share_idx, key_bytes, DA_BLOB_SHARES_INDEX_PREFIX, DA_SHARE_PREFIX,
};
use nomos_storage::{
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{relay::OutboundRelay, AsServiceId},
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::oneshot;

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
{
    let storage_relay = handle.relay().await?;

    let blob_shares =
        load_blob_shares_index::<StorageOp, DaShare>(&storage_relay, &blob_id).await?;

    let filtered_shares = blob_shares.into_iter().filter(move |idx| {
        // If requested_shares contains the index, then ignore the filter_shares
        requested_shares.contains(idx) || (return_available && !filter_shares.contains(idx))
    });

    let blob_id = Arc::new(blob_id);
    let stream = stream::iter(filtered_shares).then(move |share_idx| {
        let storage_relay = storage_relay.clone();
        let blob_id = Arc::clone(&blob_id);
        load_and_process_share::<StorageOp, DaShare>(storage_relay, blob_id, share_idx)
    });

    Ok(stream)
}

async fn load_blob_shares_index<StorageOp, DaShare>(
    storage_relay: &OutboundRelay<StorageMsg<RocksBackend<StorageOp>>>,
    blob_id: &DaShare::BlobId,
) -> Result<HashSet<DaShare::ShareIndex>, crate::http::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    DaShare::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    DaShare::ShareIndex: DeserializeOwned + Hash + Eq,
    StorageOp::Error: std::error::Error + Send + Sync + 'static,
{
    let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, blob_id);
    let (index_tx, index_rx) = oneshot::channel();
    storage_relay
        .send(StorageMsg::Load {
            key: index_key,
            reply_channel: index_tx,
        })
        .await
        .map_err(|(e, _)| Box::new(e) as crate::http::DynError)?;

    let shares_index = index_rx
        .await
        .map_err(|e| Box::new(e) as crate::http::DynError)?
        .ok_or_else(|| {
            Box::new(std::io::Error::new(
                ErrorKind::NotFound,
                "Blob index not found",
            )) as crate::http::DynError
        })?;

    StorageOp::deserialize::<HashSet<DaShare::ShareIndex>>(shares_index)
        .map_err(|e| Box::new(e) as crate::http::DynError)
}

async fn load_and_process_share<StorageOp, DaShare>(
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<StorageOp>>>,
    blob_id: Arc<<DaShare as Share>::BlobId>,
    share_idx: DaShare::ShareIndex,
) -> Result<Bytes, crate::http::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    DaShare::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    DaShare::ShareIndex: AsRef<[u8]>,
    DaShare::LightShare: Serialize + DeserializeOwned,
    StorageOp::Error: std::error::Error + Send + Sync + 'static,
{
    let share_key = key_bytes(
        DA_SHARE_PREFIX,
        create_share_idx((*blob_id).as_ref(), share_idx.as_ref()),
    );
    let (reply_tx, reply_rx) = oneshot::channel();
    storage_relay
        .send(StorageMsg::Load {
            key: share_key,
            reply_channel: reply_tx,
        })
        .await
        .map_err(|(e, _)| Box::new(e) as crate::http::DynError)?;

    let share_data = reply_rx
        .await
        .map_err(|e| Box::new(e) as crate::http::DynError)?
        .ok_or_else(|| {
            Box::new(std::io::Error::new(ErrorKind::NotFound, "Share not found"))
                as crate::http::DynError
        })?;

    let share = StorageOp::deserialize::<DaShare::LightShare>(share_data)
        .map_err(|e| Box::new(e) as crate::http::DynError)?;
    let mut json = serde_json::to_vec(&share).map_err(|e| Box::new(e) as crate::http::DynError)?;
    json.push(b'\n');

    Ok(Bytes::from(json))
}
