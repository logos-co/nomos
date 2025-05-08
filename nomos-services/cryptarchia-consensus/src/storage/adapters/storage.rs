use std::{fmt::Debug, hash::Hash, marker::PhantomData};

use nomos_core::{block::Block, header::HeaderId};
use nomos_storage::{
    api::StorageBackendApi,
    backends::{StorageBackend, StorageSerde as _},
    StorageMsg, StorageService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;

use crate::storage::StorageAdapter as StorageAdapterTrait;

pub struct StorageAdapter<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + StorageBackendApi + Send + Sync + 'static,
{
    pub storage_relay:
        OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>,
    _tx: PhantomData<Tx>,
    _blob_certificate: PhantomData<BlobCertificate>,
}

#[async_trait::async_trait]
impl<Storage, Tx, BlobCertificate, RuntimeServiceId> StorageAdapterTrait<RuntimeServiceId>
    for StorageAdapter<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + StorageBackendApi + Send + Sync + 'static,
    Tx: Clone + Debug + Eq + Hash + DeserializeOwned + Send + Sync + 'static,
    BlobCertificate: Clone + Debug + Eq + Hash + DeserializeOwned + Send + Sync + 'static,
{
    type Backend = Storage;
    type Block = Block<Tx, BlobCertificate>;

    async fn new(
        storage_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        Self {
            storage_relay,
            _tx: PhantomData,
            _blob_certificate: PhantomData,
        }
    }

    async fn get_block(&self, header_id: &HeaderId) -> Option<Self::Block> {
        let (sender, receiver) = oneshot::channel();

        self.storage_relay
            .send(StorageMsg::get_block_request(*header_id, sender))
            .await
            .unwrap();

        match receiver.await {
            Ok(Some(storage_block)) => Some(
                Storage::SerdeOperator::deserialize(storage_block)
                    .expect("Failed to deserialize block, this should not happen"),
            ),
            _ => None,
        }
    }
}
