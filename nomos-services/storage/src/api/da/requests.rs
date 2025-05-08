use tokio::sync::oneshot;

use crate::{
    StorageMsg,
    api::{StorageApiRequest, StorageBackendApi, da::StorageDaApi},
    backends::StorageBackend,
};

pub enum DaApiRequest<BlobId, Share, Commitments, ShareIdx> {
    GetLightShare {
        blob_id: BlobId,
        share_idx: ShareIdx,
        response_tx: oneshot::Sender<Option<Share>>,
    },
    StoreLightShare {
        blob_id: BlobId,
        share_idx: ShareIdx,
        light_share: Share,
    },
    StoreSharedCommitments {
        blob_id: BlobId,
        shared_commitments: Commitments,
    },
}

impl<Api: StorageBackend + StorageBackendApi> StorageMsg<Api> {
    #[must_use]
    pub const fn get_light_share_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        share_idx: <Api as StorageDaApi>::ShareIndex,
        response_tx: oneshot::Sender<Option<<Api as StorageDaApi>::Share>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetLightShare {
                blob_id,
                share_idx,
                response_tx,
            }),
        }
    }

    #[must_use]
    pub const fn store_light_share_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        share_idx: <Api as StorageDaApi>::ShareIndex,
        light_share: <Api as StorageDaApi>::Share,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreLightShare {
                blob_id,
                share_idx,
                light_share,
            }),
        }
    }

    #[must_use]
    pub const fn store_shared_commitments_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        shared_commitments: <Api as StorageDaApi>::Commitments,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreSharedCommitments {
                blob_id,
                shared_commitments,
            }),
        }
    }
}
