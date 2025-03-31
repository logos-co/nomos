use std::{collections::HashSet, fmt::Debug, hash::Hash, marker::PhantomData, path::PathBuf};

use futures::try_join;
use nomos_core::da::blob::Share;
use nomos_da_storage::rocksdb::{
    create_share_idx, key_bytes, DA_BLOB_SHARES_INDEX_PREFIX, DA_SHARED_COMMITMENTS_PREFIX,
    DA_SHARE_PREFIX,
};
use nomos_storage::{
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::oneshot;

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
        let blob_share_idx = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, blob_share_idx);
        let shared_commitments_key = key_bytes(DA_SHARED_COMMITMENTS_PREFIX, &blob_id);
        let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, &blob_id);

        let mut shares_index = self
            .try_load_shares_index(&blob_id)
            .await?
            .unwrap_or_default();
        shares_index.insert(share_idx);

        try_join!(
            self.storage_relay.send(StorageMsg::Store {
                key: share_key,
                value: S::serialize(light_share),
            }),
            self.storage_relay.send(StorageMsg::Store {
                key: shared_commitments_key,
                value: S::serialize(shared_commitments),
            }),
            self.storage_relay.send(StorageMsg::Store {
                key: index_key,
                value: S::serialize(shares_index),
            }),
        )
        .map_err(|(e, _)| DynError::from(e))?;

        Ok(())
    }
    async fn get_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
    ) -> Result<Option<<Self::Share as Share>::LightShare>, DynError> {
        let share_idx = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, share_idx);
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::Load {
                key: share_key,
                reply_channel,
            })
            .await
            .expect("Failed to send load request to storage relay");

        // TODO: Use storage backend ser/de functionality.
        //
        // Storage backend already handles ser/de, but lacks the ability to seperate
        // storage domains using prefixed keys. Once implemented Indexer and
        // Verifier can be simplified.
        reply_rx
            .await
            .map(|maybe_bytes| {
                maybe_bytes.map(|bytes| {
                    S::deserialize(bytes).expect("Blob should be deserialized from bytes")
                })
            })
            .map_err(|_| "".into())
    }
}

impl<B, S> RocksAdapter<B, S>
where
    B: Share + Clone + Send + Sync + 'static,
    B::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    B::ShareIndex: AsRef<[u8]> + DeserializeOwned + Eq + Hash + Send + Sync + 'static,
    B::LightShare: Clone + DeserializeOwned + Send + Serialize + Sync + 'static,
    B::SharesCommitments: Clone + DeserializeOwned + Send + Serialize + Sync + 'static,
    S: Send + StorageSerde + Sync + 'static,
{
    async fn try_load_shares_index(
        &self,
        blob_id: &<B as Share>::BlobId,
    ) -> Result<Option<HashSet<B::ShareIndex>>, DynError> {
        // Load the blob index. This should be most of the time only read from memory
        // because all shares are added around the same time. So we probably
        // don't need to keep additional state in the adapter to avoid reads per
        // each share
        let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, blob_id);
        let (reply_tx, reply_rx) = oneshot::channel();
        self.storage_relay
            .send(StorageMsg::Load {
                key: index_key,
                reply_channel: reply_tx,
            })
            .await
            .map_err(|(e, _)| e)?;

        let shares = reply_rx.await?.map_or_else(
            || None,
            |bytes| Some(S::deserialize(bytes).expect("Failed to deserialize shares")),
        );

        Ok(shares)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RocksAdapterSettings {
    pub blob_storage_directory: PathBuf,
}
