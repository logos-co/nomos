// std
use std::hash::Hash;
use std::marker::PhantomData;
// Crates
use nomos_core::block::Block;
use nomos_core::header::HeaderId;
use nomos_storage::backends::StorageBackend;
use nomos_storage::{StorageMsg, StorageService};
use overwatch_rs::services::relay::OutboundRelay;
use overwatch_rs::services::ServiceData;
use serde::de::DeserializeOwned;
use serde::Serialize;
// Internal
use crate::storage::StorageAdapter as StorageAdapterTrait;

pub struct StorageAdapter<Storage, Tx, BlobCertificate>
where
    Storage: StorageBackend + Send + Sync,
{
    pub storage_relay: OutboundRelay<<StorageService<Storage> as ServiceData>::Message>,
    _tx: PhantomData<Tx>,
    _blob_certificate: PhantomData<BlobCertificate>,
}

impl<Storage, Tx, BlobCertificate> StorageAdapter<Storage, Tx, BlobCertificate>
where
    Storage: StorageBackend + Send + Sync,
{
    /// Sends a store message to the storage service to retrieve a value by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The key to retrieve the value for
    ///
    /// # Returns
    ///
    /// The value for the given key. If no value is found, returns None.
    pub async fn get_value<Key, Value>(&self, key: &Key) -> Option<Value>
    where
        Key: Serialize + Send + Sync,
        Value: DeserializeOwned,
    {
        let (msg, receiver) = <StorageMsg<Storage>>::new_load_message(key);
        self.storage_relay.send(msg).await.unwrap();
        receiver.recv().await.unwrap()
    }
}

#[async_trait::async_trait]
impl<Storage, Tx, BlobCertificate> StorageAdapterTrait
    for StorageAdapter<Storage, Tx, BlobCertificate>
where
    Storage: StorageBackend + Send + Sync,
    Tx: Clone + Eq + Hash + DeserializeOwned + Send + Sync,
    BlobCertificate: Clone + Eq + Hash + DeserializeOwned + Send + Sync,
{
    type Backend = Storage;
    type Block = Block<Tx, BlobCertificate>;

    async fn new(
        storage_relay: OutboundRelay<<StorageService<Self::Backend> as ServiceData>::Message>,
    ) -> Self {
        Self {
            storage_relay,
            _tx: Default::default(),
            _blob_certificate: Default::default(),
        }
    }

    async fn get_block(&self, key: &HeaderId) -> Option<Self::Block> {
        self.get_value(key).await
    }

    async fn get_block_for_security_param(
        &self,
        current_block: Self::Block,
        security_param: &u64,
    ) -> Option<Self::Block> {
        let mut current_block = current_block;
        // TODO: This implies fetching from DB `security_param` times. We should optimize this.
        for _ in 0..*security_param {
            let parent_block_header = current_block.header().parent();
            let parent_block = self.get_block(&parent_block_header).await?;
            current_block = parent_block;
        }
        Some(current_block)
    }

    async fn save_security_block(&self, block: Self::Block) {
        let security_block_msg =
            <StorageMsg<_>>::new_store_message("security_block_header_id", block.header().id());

        if let Err((e, _msg)) = self.storage_relay.send(security_block_msg).await {
            tracing::error!("Could not send security block id to storage: {e}");
        }
    }
}
