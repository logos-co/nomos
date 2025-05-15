use std::{marker::PhantomData, path::PathBuf};

use kzgrs_backend::common::ShareIndex;
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
    share: PhantomData<B>,
}

#[async_trait::async_trait]
impl<B, S, RuntimeServiceId> DaStorageAdapter<RuntimeServiceId> for RocksAdapter<B, S>
where
    S: StorageSerde + Send + Sync + 'static,
    B: Share + DeserializeOwned + Clone + Send + Sync + 'static,
    B::LightShare: DeserializeOwned + Clone + Send + Sync + 'static,
    B::SharesCommitments: DeserializeOwned + Clone + Send + Sync + 'static,
    B::BlobId: AsRef<[u8]> + Send,
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
            share: PhantomData,
        }
    }

    async fn get_commitments(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
    ) -> Result<Option<<Self::Share as Share>::SharesCommitments>, DynError> {
        let (sc_reply_tx, sc_reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::get_shared_commitments_request(
                blob_id
                    .as_ref()
                    .try_into()
                    .expect("BlobId conversion should not fail"),
                sc_reply_tx,
            ))
            .await
            .expect("Failed to send load request to storage relay");

        let shared_commitments = sc_reply_rx.await?;
        let shared_commitments = shared_commitments
            .map(|sc| S::deserialize(sc).expect("Failed to deserialize shared commitments"));

        Ok(shared_commitments)
    }

    async fn get_light_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: ShareIndex,
    ) -> Result<Option<<Self::Share as Share>::LightShare>, DynError> {
        let blob_id = blob_id.as_ref().try_into().unwrap();
        let share_idx = share_idx.to_be_bytes();

        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::get_light_share_request(
                blob_id,
                share_idx,
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
