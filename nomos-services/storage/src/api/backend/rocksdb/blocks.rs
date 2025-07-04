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

    async fn remove_block(
        &mut self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error> {
        let encoded_header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&encoded_header_id);
        self.remove(&key).await
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{
        api::backend::rocksdb::utils::key_bytes,
        backends::{rocksdb::RocksBackendSettings, testing::NoStorageSerde},
    };

    const PREFIX: &str = "block/slot/";

    #[tokio::test]
    async fn test_blocks_by_slot() {
        let mut backend = RocksBackend::<NoStorageSerde>::new(RocksBackendSettings {
            db_path: TempDir::new().unwrap().path().to_path_buf(),
            read_only: false,
            column_family: None,
        })
        .unwrap();

        // Nothing stored yet
        assert!(backend
            .load_prefix(PREFIX.as_bytes(), None)
            .await
            .unwrap()
            .is_empty());

        store_blocks_by_slot(
            &mut backend,
            // mix the order on purpose
            &[
                (0, [0u8; 32]),
                (10, [2u8; 32]),
                (1, [1u8; 32]),
                (13, [4u8; 32]),
                (12, [3u8; 32]),
            ],
        )
        .await;
        // Store a dummy at last
        backend
            .store(
                Bytes::copy_from_slice(b"block/z/0"),
                Bytes::copy_from_slice(&[5u8; 32]),
            )
            .await
            .unwrap();

        // Block IDs should be sorted by slot
        assert_eq!(
            backend.load_prefix(PREFIX.as_bytes(), None).await.unwrap(),
            (0..=4u8)
                .map(|i| Bytes::copy_from_slice(&[i; 32]))
                .collect::<Vec<_>>()
        );

        // Iterate from a specific slot
        assert_eq!(
            backend
                .load_prefix(PREFIX.as_bytes(), Some(slot_bytes(10).as_ref()))
                .await
                .unwrap(),
            (2..=4u8)
                .map(|i| Bytes::copy_from_slice(&[i; 32]))
                .collect::<Vec<_>>()
        );

        // Iterate from a unknown slot
        assert!(backend
            .load_prefix(PREFIX.as_bytes(), Some(slot_bytes(100).as_ref()))
            .await
            .unwrap()
            .is_empty());
    }

    async fn store_blocks_by_slot<SerdeOp>(
        backend: &mut RocksBackend<SerdeOp>,
        blocks: &[(u64, [u8; 32])],
    ) where
        SerdeOp: StorageSerde + Send + Sync + 'static,
    {
        for block in blocks {
            let key = key_bytes(PREFIX, slot_bytes(block.0).as_ref());
            let value = Bytes::copy_from_slice(&block.1);
            backend.store(key, value).await.unwrap();
        }
    }

    fn slot_bytes(slot: u64) -> [u8; 8] {
        slot.to_le_bytes()
    }
}
