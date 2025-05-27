use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
};

use bytes::Bytes;
use futures::Stream;
use nomos_core::{
    block::Block,
    da::blob::{LightShare, Share},
    header::HeaderId,
};
use nomos_storage::{
    api::da::DaConverter,
    backends::{rocksdb::RocksBackend, StorageBackend, StorageSerde},
    StorageService,
};
use overwatch::services::{relay::OutboundRelay, AsServiceId, ServiceData};
use serde::{de::DeserializeOwned, Serialize};

pub mod adapters;

#[async_trait::async_trait]
pub trait StorageAdapter<StorageOp, RuntimeServiceId>
where
    StorageOp: StorageSerde + Send + Sync + 'static,
{
    type Backend: StorageBackend + Send + Sync + 'static;

    async fn get_light_share<Converter, DaShare>(
        storage_relay: OutboundRelay<
            <StorageService<RocksBackend<StorageOp>, RuntimeServiceId> as ServiceData>::Message,
        >,
        blob_id: <DaShare as Share>::BlobId,
        share_idx: <DaShare as Share>::ShareIndex,
    ) -> Result<Option<<DaShare as Share>::LightShare>, super::DynError>
    where
        DaShare: Share,
        <DaShare as Share>::BlobId: Send + Sync + 'static,
        <DaShare as Share>::ShareIndex: Send + Sync + 'static,
        <DaShare as Share>::LightShare: DeserializeOwned + Send + Sync + 'static,
        Converter: DaConverter<RocksBackend<StorageOp>, Share = DaShare> + Send + Sync + 'static;

    async fn get_shares<Converter, DaShare>(
        storage_relay: OutboundRelay<
            <StorageService<RocksBackend<StorageOp>, RuntimeServiceId> as ServiceData>::Message,
        >,
        blob_id: DaShare::BlobId,
        requested_shares: HashSet<DaShare::ShareIndex>,
        filter_shares: HashSet<DaShare::ShareIndex>,
        return_available: bool,
    ) -> Result<
        impl Stream<Item = Result<Bytes, crate::http::DynError>> + Send + Sync,
        crate::http::DynError,
    >
    where
        DaShare: Share + 'static,
        DaShare::BlobId: Clone + Send + Sync + 'static,
        DaShare::ShareIndex: DeserializeOwned + Hash + Eq + Send + Sync + 'static,
        DaShare::LightShare: LightShare<ShareIndex = DaShare::ShareIndex>
            + Serialize
            + DeserializeOwned
            + Send
            + Sync
            + 'static,
        Converter: DaConverter<RocksBackend<StorageOp>, Share = DaShare> + Send + Sync + 'static,
        RuntimeServiceId: Debug
            + Sync
            + Display
            + AsServiceId<StorageService<RocksBackend<StorageOp>, RuntimeServiceId>>;

    async fn get_shared_commitments<Converter, DaShare>(
        storage_relay: OutboundRelay<
            <StorageService<RocksBackend<StorageOp>, RuntimeServiceId> as ServiceData>::Message,
        >,
        blob_id: <DaShare as Share>::BlobId,
    ) -> Result<Option<<DaShare as Share>::SharesCommitments>, super::DynError>
    where
        DaShare: Share,
        <DaShare as Share>::BlobId: Send + Sync + 'static,
        <DaShare as Share>::SharesCommitments: DeserializeOwned + Send + Sync + 'static,
        Converter: DaConverter<RocksBackend<StorageOp>, Share = DaShare> + Send + Sync + 'static;

    async fn get_block<Tx>(
        storage_relay: OutboundRelay<
            <StorageService<RocksBackend<StorageOp>, RuntimeServiceId> as ServiceData>::Message,
        >,
        id: HeaderId,
    ) -> Result<Option<Block<Tx, kzgrs_backend::dispersal::BlobInfo>>, crate::http::DynError>
    where
        Tx: Serialize + DeserializeOwned + Clone + Eq + Hash;
}
