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
        TryFrom<Block<Tx, BlobCertificate>> + TryInto<Block<Tx, BlobCertificate>>,
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

        if let Ok(maybe_block) = receiver.await {
            let block = maybe_block?;
            block.try_into().ok()
        } else {
            tracing::error!("Failed to receive block from storage relay");
            return None;
        };

        None
    }

    async fn store_block(
        &self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), overwatch::DynError> {
        let block = block
            .try_into()
            .map_err(|_| "Failed to convert block to storage format")?;

        self.storage_relay
            .send(StorageMsg::store_block_request(header_id, block))
            .await
            .map_err(|_| "Failed to send store block request to storage relay")?;

        Ok(())
    }

    async fn remove_block(
        &self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, overwatch::DynError> {
        let (sender, receiver) = oneshot::channel();

        self.storage_relay
            .send(StorageMsg::remove_block_request(header_id, sender))
            .await
            .map_err(|_| "Failed to send remove block request to storage relay.")?;

        let Some(removed_block) = receiver
            .await
            .map_err(|_| "No block was deleted from the storage.")?
        else {
            return Ok(None);
        };

        let deserialized_block = removed_block
            .try_into()
            .map_err(|_| "Failed to convert block to storage format.")?;

        Ok(Some(deserialized_block))
    }
}
