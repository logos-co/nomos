use std::{marker::PhantomData, ops::Range, path::PathBuf};

use bytes::Bytes;
use futures::{stream::FuturesUnordered, try_join, Stream};
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::da::{
    blob::{
        info::DispersedBlobInfo,
        metadata::{Metadata, Next},
        Share as _,
    },
    BlobId,
};
use nomos_storage::{
    api::backend::rocksdb::{
        da::{DA_SHARED_COMMITMENTS_PREFIX, DA_SHARE_PREFIX, DA_VID_KEY_PREFIX},
        utils::key_bytes,
    },
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{Deserialize, Serialize};

use crate::storage::DaStorageAdapter;

pub struct RocksAdapter<S, B>
where
    S: StorageSerde + Send + Sync + 'static,
    B: DispersedBlobInfo + Metadata + Send + Sync,
{
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<S>>>,
    _vid: PhantomData<B>,
}

#[async_trait::async_trait]
impl<S, Meta, RuntimeServiceId> DaStorageAdapter<RuntimeServiceId> for RocksAdapter<S, Meta>
where
    S: StorageSerde + Send + Sync + 'static,
    Meta: DispersedBlobInfo<BlobId = BlobId> + Metadata + Send + Sync,
    Meta::Index: AsRef<[u8]> + Next + Clone + PartialOrd + Send + Sync + 'static,
    Meta::AppId: AsRef<[u8]> + Clone + Send + Sync + 'static,
{
    type Backend = RocksBackend<S>;
    type Share = DaShare;
    type Info = Meta;
    type Settings = RocksAdapterSettings;

    async fn new(
        storage_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        Self {
            storage_relay,
            _vid: PhantomData,
        }
    }

    async fn add_index(&self, info: &Self::Info) -> Result<(), DynError> {
        // Check if Info in a block is something that the node've seen before.
        let share_key = key_bytes(DA_SHARE_PREFIX, info.blob_id().as_ref());
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::LoadPrefix {
                prefix: share_key,
                reply_channel: reply_tx,
            })
            .await
            .expect("Failed to send load request to storage relay");

        // If node haven't attested this info, return early.
        if reply_rx.await?.is_empty() {
            return Ok(());
        }

        let (app_id, idx) = info.metadata();
        let vid_key = key_bytes(
            DA_VID_KEY_PREFIX,
            [app_id.clone().as_ref(), idx.as_ref()].concat(),
        );

        // We are only persisting the id part of Info, the metadata can be derived from
        // the key.
        let value = Bytes::from(info.blob_id().to_vec());

        self.storage_relay
            .send(StorageMsg::Store {
                key: vid_key,
                value,
            })
            .await
            .map_err(|(e, _)| e.into())
    }

    async fn get_range_stream(
        &self,
        app_id: <Self::Info as Metadata>::AppId,
        index_range: Range<<Self::Info as Metadata>::Index>,
    ) -> Box<dyn Stream<Item = (<Self::Info as Metadata>::Index, Vec<Self::Share>)> + Unpin + Send>
    {
        let futures = FuturesUnordered::new();

        // TODO: Using while loop here until `Step` trait is stable.
        //
        // For index_range to be used as Range with the stepping capabilities (eg. `for
        // idx in item_range`), Metadata::Index needs to implement `Step` trait,
        // which is unstable. See issue #42168 <https://github.com/rust-lang/rust/issues/42168> for more information.
        let mut current_index = index_range.start.clone();
        while current_index <= index_range.end {
            let idx = current_index.clone();
            let app_id = app_id.clone();

            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let key = key_bytes(
                DA_VID_KEY_PREFIX,
                [app_id.as_ref(), current_index.as_ref()].concat(),
            );

            self.storage_relay
                .send(StorageMsg::Load {
                    key,
                    reply_channel: reply_tx,
                })
                .await
                .expect("Failed to send load request to storage relay");

            let storage_relay = self.storage_relay.clone();
            futures.push(async move {
                let Some(id) = reply_rx.await.ok().flatten() else {
                    tracing::error!("Failed to receive storage response");
                    return (idx, Vec::new());
                };

                let Ok((shares, shared_commitments)) = try_join!(
                    async {
                        let shares_prefix_key = key_bytes(DA_SHARE_PREFIX, id.as_ref());
                        let (share_reply_tx, share_reply_rx) = tokio::sync::oneshot::channel();
                        storage_relay
                            .send(StorageMsg::LoadPrefix {
                                prefix: shares_prefix_key,
                                reply_channel: share_reply_tx,
                            })
                            .await
                            .expect("Failed to send load request to storage relay");

                        share_reply_rx.await.map_err(|e| Box::new(e) as DynError)
                    },
                    async {
                        let shared_commitments_key =
                            key_bytes(DA_SHARED_COMMITMENTS_PREFIX, id.as_ref());
                        let (shared_commitments_reply_tx, shared_commitments_reply_rx) =
                            tokio::sync::oneshot::channel();
                        storage_relay
                            .send(StorageMsg::Load {
                                key: shared_commitments_key,
                                reply_channel: shared_commitments_reply_tx,
                            })
                            .await
                            .expect("Failed to send load request to storage relay");

                        shared_commitments_reply_rx
                            .await
                            .map_err(|e| Box::new(e) as DynError)?
                            .ok_or_else(|| {
                                "Failed to receive shared commitments from storage".into()
                            })
                    }
                ) else {
                    tracing::error!("Failed to load blobs and shared commitments from storage");
                    return (idx, Vec::new());
                };

                let deserialized_shares = shares
                    .into_iter()
                    .filter_map(|bytes| S::deserialize::<DaLightShare>(bytes).ok());

                let deserialized_shared_commitments =
                    S::deserialize::<DaSharesCommitments>(shared_commitments)
                        .expect("Failed to deserialize shared commitments");

                let da_shares: Vec<_> = deserialized_shares
                    .map(|share| {
                        DaShare::from_share_and_commitments(
                            share,
                            deserialized_shared_commitments.clone(),
                        )
                    })
                    .collect();

                (idx, da_shares)
            });

            current_index = current_index.next();
        }

        Box::new(futures)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RocksAdapterSettings {
    pub blob_storage_directory: PathBuf,
}
