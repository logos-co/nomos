use std::collections::HashSet;

use async_trait::async_trait;
use bytes::Bytes;
use overwatch::DynError;
use tracing::{debug, error};

use crate::{
    api::{
        backend::rocksdb::utils::{create_share_idx, key_bytes},
        da::StorageDaApi,
    },
    backends::{StorageBackend, StorageSerde, rocksdb::RocksBackend},
};

pub const DA_VID_KEY_PREFIX: &str = "da/vid/";
pub const DA_BLOB_SHARES_INDEX_PREFIX: &str = concat!("da/verified/", "si");
pub const DA_SHARED_COMMITMENTS_PREFIX: &str = concat!("da/verified/", "sc");
pub const DA_SHARE_PREFIX: &str = concat!("da/verified/", "bl");

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageDaApi for RocksBackend<SerdeOp> {
    type BlobId = [u8; 32];
    type Share = Bytes;
    type Commitments = Bytes;
    type ShareIndex = [u8; 2];

    async fn get_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, DynError> {
        let share_idx_bytes = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, share_idx_bytes);
        let share_bytes = self.load(&share_key).await?;

        Ok(share_bytes)
    }

    async fn store_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
        light_share: Self::Share,
    ) -> Result<(), DynError> {
        let share_idx_bytes = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, share_idx_bytes);
        let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, blob_id);

        let txn = self.txn(move |db| {
            if let Err(e) = db.put(&share_key, light_share) {
                error!("Failed to store share data: {:?}", e);
                return Err(e);
            }

            let mut indices = db.get(&index_key)?.map_or_else(HashSet::new, |bytes| {
                SerdeOp::deserialize::<HashSet<[u8; 2]>>(bytes.into()).unwrap_or_else(|e| {
                    error!("Failed to deserialize indices: {:?}", e);
                    HashSet::new()
                })
            });

            indices.insert(share_idx);

            let serialized_indices = SerdeOp::serialize(indices);

            if let Err(e) = db.put(&index_key, &serialized_indices) {
                error!("Failed to store indices: {:?}", e);
                return Err(e);
            }

            Ok(None)
        });

        match self.execute(txn).await {
            Ok(_) => {
                debug!("Successfully stored light share and updated indices");
                Ok(())
            }
            Err(e) => {
                error!("Failed to execute transaction: {:?}", e);
                Err(e.into())
            }
        }
    }

    async fn store_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
        shared_commitments: Self::Commitments,
    ) -> Result<(), DynError> {
        let shared_commitments_key = key_bytes(DA_SHARED_COMMITMENTS_PREFIX, blob_id);
        self.store(shared_commitments_key, shared_commitments)
            .await?;

        Ok(())
    }
}
