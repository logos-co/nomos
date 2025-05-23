use std::{fmt::Debug, hash::Hash, marker::PhantomData, path::PathBuf};

use futures::try_join;
use nomos_core::da::blob::Share;
use nomos_storage::{
    api::da::DaConverter,
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{Deserialize, Serialize};

use crate::storage::DaStorageAdapter;

pub struct RocksAdapter<B, S, Converter>
where
    S: StorageSerde + Send + Sync + 'static,
{
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<S>>>,
    _share: PhantomData<B>,
    _converter: PhantomData<Converter>,
}

#[async_trait::async_trait]
impl<B, S, Converter, RuntimeServiceId> DaStorageAdapter<RuntimeServiceId>
    for RocksAdapter<B, S, Converter>
where
    B: Share + Clone + Send + Sync + 'static,
    B::BlobId: Clone + Send + Sync + 'static,
    B::ShareIndex: Eq + Hash + Send + Sync + 'static,
    B::LightShare: Send + Sync + 'static,
    B::SharesCommitments: Send + Sync + 'static,
    S: StorageSerde + Send + Sync + 'static,
    Converter: DaConverter<RocksBackend<S>, Share = B> + Send + Sync + 'static,
{
    type Backend = RocksBackend<S>;
    type Share = B;
    type Settings = RocksAdapterSettings;

    async fn new(
        storage_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        Self {
            storage_relay,
            _share: PhantomData,
            _converter: PhantomData,
        }
    }

    async fn add_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
        shared_commitments: <Self::Share as Share>::SharesCommitments,
        light_share: <Self::Share as Share>::LightShare,
    ) -> Result<(), DynError> {
        let store_share_msg = StorageMsg::store_light_share_request::<Converter>(
            blob_id.clone(),
            share_idx,
            light_share,
        )?;

        let store_commitments_msg =
            StorageMsg::store_shared_commitments_request::<Converter>(blob_id, shared_commitments)?;

        try_join!(
            self.storage_relay.send(store_share_msg),
            self.storage_relay.send(store_commitments_msg),
        )
        .map_err(|(e, _)| DynError::from(e))?;

        Ok(())
    }

    async fn get_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
    ) -> Result<Option<<Self::Share as Share>::LightShare>, DynError> {
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::get_light_share_request::<Converter>(
                blob_id.clone(),
                share_idx,
                reply_channel,
            )?)
            .await
            .expect("Failed to send request to storage relay");

        reply_rx
            .await
            .map_err(DynError::from)?
            .map(|data| Converter::share_from_storage(data))
            .transpose()
            .map_err(DynError::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RocksAdapterSettings {
    pub blob_storage_directory: PathBuf,
}
