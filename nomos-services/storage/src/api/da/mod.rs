use std::error::Error;

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

    async fn store_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
        light_share: Self::Share,
    ) -> Result<(), Self::Error>;

    async fn store_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
        shared_commitments: Self::Commitments,
    ) -> Result<(), Self::Error>;
}
