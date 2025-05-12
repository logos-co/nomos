use std::{hash::Hash, marker::PhantomData};

use nomos_core::{block::Block, header::HeaderId};
use nomos_storage::{
    api::chain::StorageChainApi, backends::StorageBackend, StorageMsg, StorageService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::oneshot;

use crate::storage::StorageAdapter as StorageAdapterTrait;

pub struct StorageAdapter<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
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
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        From<Block<Tx, BlobCertificate>> + Into<Block<Tx, BlobCertificate>>,
    Tx: Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    BlobCertificate: Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
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

        receiver
            .await
            .map(|maybe_block| maybe_block.map(|storage_block| storage_block.into()))
            .unwrap_or(None);

        None
    }

    async fn store_block(
        &self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), overwatch::DynError> {
        self.storage_relay
            .send(StorageMsg::store_block_request(header_id, block.into()))
            .await
            .expect("Failed to send store block request");

        Ok(())
    }
}
