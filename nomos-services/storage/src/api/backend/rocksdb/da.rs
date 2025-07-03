use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use bytes::Bytes;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use nomos_core::{block::BlockNumber, da::BlobId};
use rocksdb::Error;
use tracing::{debug, error};

use crate::{
    api::{
        backend::rocksdb::utils::{create_share_idx, key_bytes},
        da::StorageDaApi,
    },
    backends::{rocksdb::RocksBackend, StorageBackend as _, StorageSerde},
};

pub const DA_VID_KEY_PREFIX: &str = "da/vid/";
pub const DA_BLOB_SHARES_INDEX_PREFIX: &str = concat!("da/verified/", "si");
pub const DA_SHARED_COMMITMENTS_PREFIX: &str = concat!("da/verified/", "sc");
pub const DA_SHARE_PREFIX: &str = concat!("da/verified/", "bl");
pub const DA_ASSIGNATIONS_PREFIX: &str = concat!("da/membership/", "assignations");
pub const DA_ADDRESSBOOK_PREFIX: &str = concat!("da/membership/", "addressbook");

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageDaApi for RocksBackend<SerdeOp> {
    type Error = Error;
    type BlobId = BlobId;
    type Share = Bytes;
    type Commitments = Bytes;
    type ShareIndex = [u8; 2];
    type NetworkId = u16;
    type Id = PeerId;

    async fn get_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, Self::Error> {
        let share_idx_bytes = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, share_idx_bytes);
        let share_bytes = self.load(&share_key).await?;
        Ok(share_bytes)
    }

    async fn get_blob_light_shares(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<Vec<Self::Share>>, Self::Error> {
        let shares_prefix_key = key_bytes(DA_SHARE_PREFIX, blob_id.as_ref());
        let shares_bytes = self.load_prefix(&shares_prefix_key).await?;
        if shares_bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(shares_bytes))
    }

    async fn get_blob_share_indices(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<HashSet<Self::ShareIndex>>, Self::Error> {
        let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, blob_id.as_ref());
        let indices_bytes = self.load(&index_key).await?;
        let indices = indices_bytes.map(|bytes| {
            SerdeOp::deserialize::<HashSet<Self::ShareIndex>>(bytes).unwrap_or_else(|e| {
                error!("Failed to deserialize indices: {:?}", e);
                HashSet::new()
            })
        });
        Ok(indices)
    }

    async fn store_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
        light_share: Self::Share,
    ) -> Result<(), Self::Error> {
        let share_idx_bytes = create_share_idx(blob_id.as_ref(), share_idx.as_ref());
        let share_key = key_bytes(DA_SHARE_PREFIX, share_idx_bytes);
        let index_key = key_bytes(DA_BLOB_SHARES_INDEX_PREFIX, blob_id.as_ref());

        let txn = self.txn(move |db| {
            if let Err(e) = db.put(&share_key, &light_share) {
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
                Err(e)
            }
        }
    }

    async fn get_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<Self::Commitments>, Self::Error> {
        let commitments_key = key_bytes(DA_SHARED_COMMITMENTS_PREFIX, blob_id.as_ref());
        let commitments_bytes = self.load(&commitments_key).await?;
        Ok(commitments_bytes)
    }

    async fn store_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
        shared_commitments: Self::Commitments,
    ) -> Result<(), Self::Error> {
        let commitments_key = key_bytes(DA_SHARED_COMMITMENTS_PREFIX, blob_id.as_ref());
        self.store(commitments_key, shared_commitments).await
    }

    async fn store_assignations(
        &mut self,
        block_number: BlockNumber,
        assignations: HashMap<Self::NetworkId, HashSet<Self::Id>>,
        addressbook: HashMap<Self::Id, Multiaddr>,
    ) -> Result<(), Self::Error> {
        let block_bytes = block_number.to_be_bytes();
        let assignations_key = key_bytes(DA_ASSIGNATIONS_PREFIX, block_bytes);
        let addressbook_key = key_bytes(DA_ADDRESSBOOK_PREFIX, block_bytes);

        let serialized_assignations = SerdeOp::serialize(assignations);
        let serialized_addressbook = SerdeOp::serialize(addressbook);

        let txn = self.txn(move |db| {
            if let Err(e) = db.put(&assignations_key, &serialized_assignations) {
                error!(
                    "Failed to store assignations for block {}: {:?}",
                    block_number, e
                );
                return Err(e);
            }

            if let Err(e) = db.put(&addressbook_key, &serialized_addressbook) {
                error!(
                    "Failed to store addressbook for block {}: {:?}",
                    block_number, e
                );
                return Err(e);
            }

            Ok(None)
        });

        match self.execute(txn).await {
            Ok(_) => {
                debug!(
                    "Successfully stored assignations and addressbook for block {}",
                    block_number
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to execute transaction for block {}: {:?}",
                    block_number, e
                );
                Err(e)
            }
        }
    }

    async fn get_assignations(
        &mut self,
        block_number: BlockNumber,
    ) -> Result<
        (
            HashMap<Self::NetworkId, HashSet<Self::Id>>,
            HashMap<Self::Id, Multiaddr>,
        ),
        Self::Error,
    > {
        let block_bytes = block_number.to_be_bytes();
        let assignations_key = key_bytes(DA_ASSIGNATIONS_PREFIX, block_bytes);
        let addressbook_key = key_bytes(DA_ADDRESSBOOK_PREFIX, block_bytes);

        // Load both pieces of data
        let assignations_bytes = self.load(&assignations_key).await?;
        let addressbook_bytes = self.load(&addressbook_key).await?;

        match (assignations_bytes, addressbook_bytes) {
            (Some(assignations_data), Some(addressbook_data)) => {
                // Deserialize both - follow existing pattern with unwrap_or_else
                let assignations = SerdeOp::deserialize::<
                    HashMap<Self::NetworkId, HashSet<Self::Id>>,
                >(assignations_data)
                .unwrap_or_else(|e| {
                    error!(
                        "Failed to deserialize assignations for block {}: {:?}",
                        block_number, e
                    );
                    HashMap::new()
                });

                let addressbook =
                    SerdeOp::deserialize::<HashMap<Self::Id, Multiaddr>>(addressbook_data)
                        .unwrap_or_else(|e| {
                            error!(
                                "Failed to deserialize addressbook for block {}: {:?}",
                                block_number, e
                            );
                            HashMap::new()
                        });

                debug!(
                    "Successfully loaded assignations and addressbook for block {}",
                    block_number
                );
                Ok((assignations, addressbook))
            }
            (None, None) => {
                // No data found for this block number
                debug!("No membership data found for block {}", block_number);
                Ok((HashMap::new(), HashMap::new()))
            }
            _ => {
                // Partial data - log error but return what we can
                error!(
                "Inconsistent membership data for block {}: missing assignations or addressbook", 
                block_number
            );
                // Return empty maps like we do for missing data
                Ok((HashMap::new(), HashMap::new()))
            }
        }
    }
}
