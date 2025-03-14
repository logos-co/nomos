pub mod adapters;

use kzgrs_backend::common::ColumnIndex;
use nomos_core::da::blob::Blob;
use nomos_storage::{backends::StorageBackend, StorageService};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

#[async_trait::async_trait]
pub trait DaStorageAdapter {
    type Backend: StorageBackend + Send + Sync + 'static;
    type Settings: Clone + Send + Sync + 'static;
    type Blob: Blob + Clone;
    async fn new(
        storage_relay: OutboundRelay<<StorageService<Self::Backend> as ServiceData>::Message>,
    ) -> Self;

    async fn get_commitments(
        &self,
        blob_id: <Self::Blob as Blob>::BlobId,
    ) -> Result<Option<<Self::Blob as Blob>::SharedCommitments>, DynError>;

    async fn get_light_blob(
        &self,
        blob_id: <Self::Blob as Blob>::BlobId,
        column_idx: ColumnIndex,
    ) -> Result<Option<<Self::Blob as Blob>::LightBlob>, DynError>;
}
