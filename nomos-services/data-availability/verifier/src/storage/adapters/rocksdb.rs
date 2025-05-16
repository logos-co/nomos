use std::{fmt::Debug, hash::Hash, marker::PhantomData, path::PathBuf};

use futures::try_join;
use nomos_core::da::blob::Share;
use nomos_storage::{
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::storage::DaStorageAdapter;

pub struct RocksAdapter<B, S>
where
    S: StorageSerde + Send + Sync + 'static,
{
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<S>>>,
    _share: PhantomData<B>,
}

#[async_trait::async_trait]
impl<B, S, RuntimeServiceId> DaStorageAdapter<RuntimeServiceId> for RocksAdapter<B, S>
where
    B: Share + Clone + Send + Sync + 'static,
    B::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    B::ShareIndex: AsRef<[u8]> + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    B::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    B::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    S: StorageSerde + Send + Sync + 'static,
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
        }
    }

    async fn add_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
        shared_commitments: <Self::Share as Share>::SharesCommitments,
        light_share: <Self::Share as Share>::LightShare,
    ) -> Result<(), DynError> {
        let store_share_msg = StorageMsg::store_light_share_request(
            blob_id
                .as_ref()
                .try_into()
                .expect("BlobId conversion should not fail"),
            share_idx
                .as_ref()
                .try_into()
                .expect("ShareIndex conversion should not fail"),
            S::serialize(light_share),
        );

        let store_commitments_msg = StorageMsg::store_shared_commitments_request(
            blob_id
                .as_ref()
                .try_into()
                .expect("BlobId conversion should not fail"),
            S::serialize(shared_commitments.clone()),
        );

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
            .send(StorageMsg::get_light_share_request(
                blob_id
                    .as_ref()
                    .try_into()
                    .expect("BlobId conversion should not fail"),
                share_idx
                    .as_ref()
                    .try_into()
                    .expect("ShareIndex conversion should not fail"),
                reply_channel,
            ))
            .await
            .expect("Failed to send request to storage relay");

        reply_rx
            .await
            .map(|maybe_share| {
                maybe_share
                    .map(|share| S::deserialize(share).expect("Failed to deserialize light share"))
            })
            .map_err(|_| "Failed to receive response from storage".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RocksAdapterSettings {
    pub blob_storage_directory: PathBuf,
}
