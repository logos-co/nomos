pub mod adapters;

use nomos_core::da::blob::Share;
use nomos_storage::{StorageService, api::StorageBackendApi, backends::StorageBackend};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};

#[async_trait::async_trait]
pub trait DaStorageAdapter<RuntimeServiceId> {
    type Backend: StorageBackend + StorageBackendApi + Send + Sync + 'static;
    type Settings: Clone;
    type Share: Share + Clone;

    async fn new(
        storage_relay: OutboundRelay<
            <StorageService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    async fn add_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
        commitments: <Self::Share as Share>::SharesCommitments,
        light_share: <Self::Share as Share>::LightShare,
    ) -> Result<(), DynError>;

    async fn get_share(
        &self,
        blob_id: <Self::Share as Share>::BlobId,
        share_idx: <Self::Share as Share>::ShareIndex,
    ) -> Result<Option<<Self::Share as Share>::LightShare>, DynError>;
}
