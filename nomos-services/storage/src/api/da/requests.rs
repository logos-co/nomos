use tokio::sync::oneshot::Sender;

use crate::{
    api::{da::StorageDaApi, StorageApiRequest, StorageBackendApi, StorageOperation},
    backends::StorageBackend,
    StorageMsg, StorageServiceError,
};

pub enum DaApiRequest<B: StorageBackend> {
    GetLightShare {
        blob_id: <B as StorageDaApi>::BlobId,
        share_idx: <B as StorageDaApi>::ShareIndex,
        response_tx: Sender<Option<<B as StorageDaApi>::Share>>,
    },
    StoreLightShare {
        blob_id: <B as StorageDaApi>::BlobId,
        share_idx: <B as StorageDaApi>::ShareIndex,
        light_share: <B as StorageDaApi>::Share,
    },
    StoreSharedCommitments {
        blob_id: <B as StorageDaApi>::BlobId,
        shared_commitments: <B as StorageDaApi>::Commitments,
    },
}

impl<B> StorageOperation<B> for DaApiRequest<B>
where
    B: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError> {
        match self {
            Self::GetLightShare {
                blob_id,
                share_idx,
                response_tx,
            } => handle_get_light_share(backend, blob_id, share_idx, response_tx).await,
            Self::StoreLightShare {
                blob_id,
                share_idx,
                light_share,
            } => handle_store_light_share(backend, blob_id, share_idx, light_share).await,
            Self::StoreSharedCommitments {
                blob_id,
                shared_commitments,
            } => handle_store_shared_commitments(backend, blob_id, shared_commitments).await,
        }
    }
}

async fn handle_get_light_share<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    share_idx: B::ShareIndex,
    response_tx: Sender<Option<B::Share>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_light_share(blob_id, share_idx)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get light share request".to_owned(),
        });
    }

    Ok(())
}

async fn handle_store_light_share<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    share_idx: B::ShareIndex,
    light_share: B::Share,
) -> Result<(), StorageServiceError> {
    backend
        .store_light_share(blob_id, share_idx, light_share)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_store_shared_commitments<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    shared_commitments: B::Commitments,
) -> Result<(), StorageServiceError> {
    backend
        .store_shared_commitments(blob_id, shared_commitments)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

impl<Api: StorageBackend> StorageMsg<Api> {
    #[must_use]
    pub const fn get_light_share_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        share_idx: <Api as StorageDaApi>::ShareIndex,
        response_tx: Sender<Option<<Api as StorageDaApi>::Share>>,
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
