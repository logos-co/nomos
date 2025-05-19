use std::{collections::HashSet, error::Error};

use async_trait::async_trait;

pub mod requests;

#[async_trait]
pub trait StorageDaApi {
    type Error: Error + Send + Sync + 'static;
    type BlobId: Send + Sync;
    type Share: Send + Sync;
    type Commitments: Send + Sync;
    type ShareIndex: Send + Sync;

    async fn get_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, Self::Error>;

    async fn get_blob_light_shares(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<Vec<Self::Share>>, Self::Error>;

    async fn get_blob_share_indices(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<HashSet<Self::ShareIndex>>, Self::Error>;

    async fn store_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
        light_share: Self::Share,
    ) -> Result<(), Self::Error>;

    async fn get_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
    ) -> Result<Option<Self::Commitments>, Self::Error>;

    async fn store_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
        shared_commitments: Self::Commitments,
    ) -> Result<(), Self::Error>;
}
