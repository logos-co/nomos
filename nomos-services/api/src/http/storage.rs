use std::fmt::{Debug, Display};

use bytes::Bytes;
use nomos_core::{block::Block, da::blob::Share, header::HeaderId};
use nomos_storage::{
    api::da::StorageDaApi,
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::services::AsServiceId;
use serde::{de::DeserializeOwned, Serialize};

use crate::{http::da_shares::DaStorageBackend, wait_with_timeout};

pub async fn block_req<S, Tx, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    id: HeaderId,
) -> Result<Option<Block<Tx, kzgrs_backend::dispersal::BlobInfo>>, super::DynError>
where
    Tx: serde::Serialize + DeserializeOwned + Clone + Eq + core::hash::Hash,
    S: StorageSerde + Send + Sync + 'static,
    RuntimeServiceId:
        AsServiceId<StorageService<RocksBackend<S>, RuntimeServiceId>> + Debug + Sync + Display,
{
    let relay = handle.relay().await?;
    let key: [u8; 32] = id.into();
    let (msg, receiver) = StorageMsg::new_load_message(Bytes::copy_from_slice(&key));
    relay.send(msg).await.map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver.recv(),
        "Timeout while waiting for block".to_owned(),
    )
    .await
}

pub async fn get_shared_commitments<StorageOp, DaShare, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    blob_id: <DaShare as Share>::BlobId,
) -> Result<Option<<DaShare as Share>::SharesCommitments>, super::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    <DaShare as Share>::BlobId:
        AsRef<[u8]> + serde::Serialize + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::SharesCommitments:
        serde::Serialize + DeserializeOwned + Send + Sync + 'static,
    <StorageOp as StorageSerde>::Error: Send + Sync,
    RuntimeServiceId: AsServiceId<StorageService<RocksBackend<StorageOp>, RuntimeServiceId>>
        + Debug
        + Sync
        + Display,
    // Service and storage layer types conversions
    <DaStorageBackend<StorageOp> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
{
    let relay = handle.relay().await?;

    let (reply_tx, reply_rcv) = tokio::sync::oneshot::channel();
    relay
        .send(StorageMsg::get_shared_commitments_request(
            blob_id.into(),
            reply_tx,
        ))
        .await
        .map_err(|(e, _)| e)?;

    let result = wait_with_timeout(
        reply_rcv,
        "Timeout while waiting for shared commitments".to_owned(),
    )
    .await?;

    result
        .map(|data| StorageOp::deserialize(data))
        .transpose()
        .map_err(super::DynError::from)
}

pub async fn get_light_share<StorageOp, DaShare, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    blob_id: <DaShare as Share>::BlobId,
    share_idx: <DaShare as Share>::ShareIndex,
) -> Result<Option<<DaShare as Share>::LightShare>, super::DynError>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
    DaShare: Share,
    <DaShare as Share>::BlobId: AsRef<[u8]> + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: AsRef<[u8]> + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::LightShare: Serialize + DeserializeOwned,
    <StorageOp as StorageSerde>::Error: Send + Sync,
    RuntimeServiceId: AsServiceId<StorageService<RocksBackend<StorageOp>, RuntimeServiceId>>
        + Debug
        + Sync
        + Display,
    // Service and storage layer types conversions
    <DaStorageBackend<StorageOp> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
    <DaStorageBackend<StorageOp> as StorageDaApi>::ShareIndex: From<DaShare::ShareIndex>,
{
    let relay = handle.relay().await?;

    let (reply_tx, reply_rcv) = tokio::sync::oneshot::channel();
    relay
        .send(StorageMsg::get_light_share_request(
            blob_id.into(),
            share_idx.into(),
            reply_tx,
        ))
        .await
        .map_err(|(e, _)| e)?;

    let result = wait_with_timeout(
        reply_rcv,
        "Timeout while waiting for light share".to_owned(),
    )
    .await?;

    result
        .map(|data| StorageOp::deserialize(data))
        .transpose()
        .map_err(super::DynError::from)
}
