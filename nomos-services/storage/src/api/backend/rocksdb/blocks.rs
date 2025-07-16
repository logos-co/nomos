use std::{collections::BTreeMap, num::NonZeroUsize, ops::RangeInclusive};

use async_trait::async_trait;
use bytes::Bytes;
use cryptarchia_engine::Slot;
use nomos_core::header::HeaderId;
use rocksdb::WriteBatch;

use crate::{
    api::{
        backend::rocksdb::{utils::key_bytes, Error},
        chain::StorageChainApi,
    },
    backends::{rocksdb::RocksBackend, StorageBackend as _, StorageSerde},
};

const IMMUTABLE_BLOCK_PREFIX: &str = "immutable_block/slot/";

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageChainApi for RocksBackend<SerdeOp> {
    type Error = Error;
    type Block = Bytes;
    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.load(&key).await.map_err(Into::into)
    }

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), Self::Error> {
        let header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&header_id);
        self.store(key, block).await.map_err(Into::into)
    }

    async fn remove_block(
        &mut self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error> {
        let encoded_header_id: [u8; 32] = header_id.into();
        let key = Bytes::copy_from_slice(&encoded_header_id);
        self.remove(&key).await.map_err(Into::into)
    }

    async fn store_immutable_block_ids(
        &mut self,
        ids: BTreeMap<Slot, HeaderId>,
    ) -> Result<(), Self::Error> {
        let txn = self.txn(move |db| {
            let mut batch = WriteBatch::default();
            for (slot, header_id) in ids {
                let key = key_bytes(IMMUTABLE_BLOCK_PREFIX, slot.to_be_bytes());
                let header_id: [u8; 32] = header_id.into();
                batch.put(key, Bytes::copy_from_slice(&header_id));
            }
            db.write(batch)?;
            Ok(None)
        });
        let _ = self.execute(txn).await?;

        Ok(())
    }

    async fn get_immutable_block_id(
        &mut self,
        slot: Slot,
    ) -> Result<Option<HeaderId>, Self::Error> {
        let key = key_bytes(IMMUTABLE_BLOCK_PREFIX, slot.to_be_bytes());
        self.load(&key)
            .await?
            .map(|bytes| bytes.as_ref().try_into().map_err(Into::into))
            .transpose()
    }

    async fn scan_immutable_block_ids(
        &mut self,
        slot_range: RangeInclusive<Slot>,
        limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, Self::Error> {
        let start_key = slot_range.start().to_be_bytes();
        let end_key = slot_range.end().to_be_bytes();
        let result = self
            .load_prefix(
                IMMUTABLE_BLOCK_PREFIX.as_ref(),
                Some(&start_key),
                Some(&end_key),
                Some(limit),
            )
            .await?;

        result
            .into_iter()
            .map(|bytes| bytes.as_ref().try_into().map_err(Into::into))
            .collect::<Result<Vec<HeaderId>, Error>>()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::backends::{rocksdb::RocksBackendSettings, testing::NoStorageSerde};

    #[tokio::test]
    async fn immutable_block_ids() {
        let temp_dir = TempDir::new().unwrap();
        let mut backend = RocksBackend::<NoStorageSerde>::new(RocksBackendSettings {
            db_path: temp_dir.path().to_path_buf(),
            read_only: false,
            column_family: None,
        })
        .unwrap();

        // Store
        backend
            .store_immutable_block_ids(
                [(0.into(), [0u8; 32].into()), (1.into(), [1u8; 32].into())].into(),
            )
            .await
            .unwrap();

        // Get
        assert_eq!(
            backend.get_immutable_block_id(0.into()).await.unwrap(),
            Some([0u8; 32].into())
        );
        assert_eq!(
            backend.get_immutable_block_id(1.into()).await.unwrap(),
            Some([1u8; 32].into())
        );

        // Scan
        assert_eq!(
            backend
                .scan_immutable_block_ids(
                    RangeInclusive::new(0.into(), 1.into()),
                    NonZeroUsize::new(2).unwrap()
                )
                .await
                .unwrap(),
            vec![[0u8; 32].into(), [1u8; 32].into()]
        );
        assert_eq!(
            backend
                .scan_immutable_block_ids(
                    RangeInclusive::new(0.into(), 1.into()),
                    NonZeroUsize::new(1).unwrap()
                )
                .await
                .unwrap(),
            vec![[0u8; 32].into()]
        );
        assert_eq!(
            backend
                .scan_immutable_block_ids(
                    RangeInclusive::new(0.into(), 0.into()),
                    NonZeroUsize::new(2).unwrap()
                )
                .await
                .unwrap(),
            vec![[0u8; 32].into()]
        );
        assert_eq!(
            backend
                .scan_immutable_block_ids(
                    RangeInclusive::new(1.into(), 2.into()),
                    NonZeroUsize::new(2).unwrap()
                )
                .await
                .unwrap(),
            vec![[1u8; 32].into()]
        );
    }
}
