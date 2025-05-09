use std::fmt::Debug;

use async_trait::async_trait;
use overwatch::DynError;

pub mod requests;

#[async_trait]
pub trait StorageDaApi {
    type BlobId: Debug + Send + Sync + 'static;
    type Share: Debug + Send + Sync + 'static;
    type Commitments: Debug + Send + Sync + 'static;
    type ShareIndex: Debug + Send + Sync + 'static;

    async fn get_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
    ) -> Result<Option<Self::Share>, DynError>;

    async fn store_light_share(
        &mut self,
        blob_id: Self::BlobId,
        share_idx: Self::ShareIndex,
        light_share: Self::Share,
    ) -> Result<(), DynError>;

    async fn store_shared_commitments(
        &mut self,
        blob_id: Self::BlobId,
        shared_commitments: Self::Commitments,
    ) -> Result<(), DynError>;
}
