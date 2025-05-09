pub mod adapters;

use kzgrs_backend::common::ShareIndex;
use nomos_core::da::blob::Share;
use nomos_storage::{backends::StorageBackend, StorageService};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

#[async_trait::async_trait]
pub trait DaStorageAdapter<RuntimeServiceId> {
    type Backend: StorageBackend + Send + Sync + 'static;
    type Settings: Clone + Send + Sync + 'static;
    type Share: Share + Clone;
    async fn new(
        storage_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    async fn get_commitments(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
    ) -> Result<Option<<Self::Share as Share>::SharesCommitments>, DynError>;

    async fn get_light_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: ShareIndex,
    ) -> Result<Option<<Self::Share as Share>::LightShare>, DynError>;
}
