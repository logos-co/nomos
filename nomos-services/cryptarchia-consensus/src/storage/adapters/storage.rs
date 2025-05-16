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

    async fn remove_block(&self, header_id: HeaderId) -> Result<Self::Block, overwatch::DynError> {
        let (sender, receiver) = oneshot::channel();

        self.storage_relay
            .send(StorageMsg::remove_block_request(header_id, sender))
            .await
            .map_err(|_| "Failed to send remove block request to storage relay")?;

        let maybe_removed_block = receiver
            .await
            .map_err(|_| "Failed to receive removed block from storage relay")?;
        let removed_block = maybe_removed_block?;

        let deserialized_block = removed_block
            .try_into()
            .map_err(|_| "Failed to convert block to storage format")?;

        Ok(deserialized_block)
    }

    async fn remove_blocks<Headers>(
        &self,
        header_ids: Headers,
    ) -> Result<impl Iterator<Item = Self::Block>, overwatch::DynError>
    where
        Headers: Iterator<Item = HeaderId> + Send + Sync,
    {
        let (sender, receiver) = oneshot::channel();

        self.storage_relay
            .send(StorageMsg::remove_blocks_request(
                header_ids.collect(),
                sender,
            ))
            .await
            .map_err(|_| "Failed to send remove blocks request to storage relay")?;

        let maybe_removed_blocks = receiver
            .await
            .map_err(|_| "Failed to receive removed blocks from storage relay")?;
        let removed_blocks = maybe_removed_blocks?;

        let deserialized_blocks = removed_blocks
            .into_iter()
            .map(|b| -> Result<_, overwatch::DynError> {
                let deserialized_block = b
                    .try_into()
                    .map_err(|_| "Failed to convert block to storage format")?;
                Ok(deserialized_block)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(deserialized_blocks.into_iter())
    }
}
