use std::{collections::HashSet, error::Error};

use async_trait::async_trait;
use nomos_core::da::blob::Share;

pub mod requests;

type ServiceBlobId<Converter, Backend> =
    <<Converter as DaConverter<Backend>>::Share as Share>::BlobId;

type ServiceShareIndex<Converter, Backend> =
    <<Converter as DaConverter<Backend>>::Share as Share>::ShareIndex;

type ServiceLightShare<Converter, Backend> =
    <<Converter as DaConverter<Backend>>::Share as Share>::LightShare;

type ServiceSharedCommitments<Converter, Backend> =
    <<Converter as DaConverter<Backend>>::Share as Share>::SharesCommitments;

pub trait DaConverter<Backend: StorageDaApi> {
    type Share: Share;
    type Error: Error + Send + Sync + 'static;

    fn blob_id_to_storage(
        blob_id: ServiceBlobId<Self, Backend>,
    ) -> Result<Backend::BlobId, Self::Error>;

    fn blob_id_from_storage(
        blob_id: Backend::BlobId,
    ) -> Result<ServiceBlobId<Self, Backend>, Self::Error>;

    fn share_index_to_storage(
        share_index: ServiceShareIndex<Self, Backend>,
    ) -> Result<Backend::ShareIndex, Self::Error>;

    fn share_index_from_storage(
        share_index: Backend::ShareIndex,
    ) -> Result<ServiceShareIndex<Self, Backend>, Self::Error>;

    fn share_to_storage(
        service_share: ServiceLightShare<Self, Backend>,
    ) -> Result<Backend::Share, Self::Error>;

    fn share_from_storage(
        backend_share: Backend::Share,
    ) -> Result<ServiceLightShare<Self, Backend>, Self::Error>;

    fn commitments_to_storage(
        service_commitments: ServiceSharedCommitments<Self, Backend>,
    ) -> Result<Backend::Commitments, Self::Error>;

    fn commitments_from_storage(
        backend_commitments: Backend::Commitments,
    ) -> Result<ServiceSharedCommitments<Self, Backend>, Self::Error>;
}

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
