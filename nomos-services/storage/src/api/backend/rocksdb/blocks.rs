use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;

use crate::{
    api::chain::StorageChainApi,
    backends::{
        rocksdb::{Error, RocksBackend},
        StorageBackend as _, StorageSerde,
    },
};

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for RocksBackend<SerdeOp> {
    type Error = Error;
    type Block = Bytes;
    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.load(&key).await
    }

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.store(key, block).await
    }

    async fn remove_block(&mut self, header_id: HeaderId) -> Result<Self::Block, Self::Error> {
        let encoded_header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&encoded_header_id);
        self.remove(&key)
            .await?
            .ok_or_else(|| Error::Other("Block {encoded_header_id} not found".to_owned()))
    }

    async fn remove_blocks<Headers>(
        &mut self,
        header_ids: Headers,
    ) -> Result<impl Iterator<Item = Self::Block>, Self::Error>
    where
        Headers: Iterator<Item = HeaderId> + Send + Sync,
    {
        let mut blocks = Vec::new();
        for header_id in header_ids {
            // We cannot process multiple blocks in parallel since we cannot mutably borrow
            // more than once. This should be replaced with a RocksDB batch
            // operation in the future.
            blocks.push(self.remove_block(header_id).await?);
        }

        Ok(blocks.into_iter())
    }
}
