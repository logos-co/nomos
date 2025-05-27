use std::{marker::PhantomData, ops::Range, path::PathBuf};

use bytes::Bytes;
use futures::{stream::FuturesUnordered, try_join, Stream};
use kzgrs_backend::common::share::DaShare;
use nomos_core::da::{
    blob::{
        info::DispersedBlobInfo,
        metadata::{Metadata, Next},
        Share as _,
    },
    BlobId,
};
use nomos_storage::{
    api::{
        backend::rocksdb::{da::DA_VID_KEY_PREFIX, utils::key_bytes},
        da::DaConverter,
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

pub struct RocksAdapter<S, B, Converter>
where
    S: StorageSerde + Send + Sync + 'static,
    B: DispersedBlobInfo + Metadata + Send + Sync,
{
    storage_relay: OutboundRelay<StorageMsg<RocksBackend<S>>>,
    _vid: PhantomData<B>,
    _converter: PhantomData<Converter>,
}

#[async_trait::async_trait]
impl<S, Meta, Converter, RuntimeServiceId> DaStorageAdapter<RuntimeServiceId>
    for RocksAdapter<S, Meta, Converter>
where
    S: StorageSerde + Send + Sync + 'static,
    Converter: DaConverter<RocksBackend<S>, Share = DaShare> + Send + Sync + 'static,
    Meta: DispersedBlobInfo<BlobId = BlobId> + Send + Sync,
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
            _converter: PhantomData,
        }
    }

    async fn add_index(&self, info: &Self::Info) -> Result<(), DynError> {
        // Check if Info in a block is something that the node've seen before.
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.storage_relay
            .send(StorageMsg::get_blob_light_shares_request::<Converter>(
                info.blob_id(),
                reply_tx,
            )?)
            .await
            .expect("Failed to send load request to storage relay");

        // If node haven't attested this info, return early.
        let indexes = reply_rx.await?;
        if indexes.is_none() || indexes.unwrap().is_empty() {
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
                        let (share_reply_tx, share_reply_rx) = tokio::sync::oneshot::channel();
                        storage_relay
                            .send(StorageMsg::get_blob_light_shares_request::<Converter>(
                                id.as_ref().try_into().expect("Failed to convert blob id"),
                                share_reply_tx,
                            )?)
                            .await
                            .expect("Failed to send load request to storage relay");

                        share_reply_rx.await.map_err(|e| Box::new(e) as DynError)
                    },
                    async {
                        let (shared_commitments_reply_tx, shared_commitments_reply_rx) =
                            tokio::sync::oneshot::channel();
                        storage_relay
                            .send(StorageMsg::get_shared_commitments_request::<Converter>(
                                id.as_ref().try_into().expect("Failed to convert blob id"),
                                shared_commitments_reply_tx,
                            )?)
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
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|bytes| Converter::share_from_storage(bytes).ok());

                let deserialized_shared_commitments =
                    Converter::commitments_from_storage(shared_commitments)
                        .ok()
                        .unwrap_or_default();

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
