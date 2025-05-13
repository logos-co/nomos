use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;
use rocksdb::Error;

use crate::{
    api::chain::StorageChainApi,
    backends::{rocksdb::RocksBackend, StorageBackend as _, StorageSerde},
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
}
