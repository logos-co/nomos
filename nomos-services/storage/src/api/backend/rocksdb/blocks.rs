use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;
use overwatch::DynError;

use crate::{
    api::chain::StorageChainApi,
    backends::{StorageBackend, StorageSerde, rocksdb::RocksBackend},
};

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for RocksBackend<SerdeOp> {
    type HeaderId = HeaderId;
    type Block = Bytes;
    async fn get_block(
        &mut self,
        header_id: Self::HeaderId,
    ) -> Result<Option<Self::Block>, DynError> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        Ok(self.load(&key).await?)
    }

    async fn store_block(
        &mut self,
        header_id: Self::HeaderId,
        block: Self::Block,
    ) -> Result<(), DynError> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        Ok(self.store(key, block).await?)
    }
}
