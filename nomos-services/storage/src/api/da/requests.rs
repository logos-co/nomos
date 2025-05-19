use std::collections::HashSet;

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
    GetLightShareIndexes {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<HashSet<<B as StorageDaApi>::ShareIndex>>>,
    },
    GetBlobLightShares {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<Vec<<B as StorageDaApi>::Share>>>,
    },
    StoreLightShare {
        blob_id: <B as StorageDaApi>::BlobId,
        share_idx: <B as StorageDaApi>::ShareIndex,
        light_share: <B as StorageDaApi>::Share,
    },
    GetSharedCommitments {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<<B as StorageDaApi>::Commitments>>,
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
            Self::GetSharedCommitments {
                blob_id,
                response_tx,
            } => handle_get_shared_commitments(backend, blob_id, response_tx).await,
            Self::StoreSharedCommitments {
                blob_id,
                shared_commitments,
            } => handle_store_shared_commitments(backend, blob_id, shared_commitments).await,
            Self::GetLightShareIndexes {
                blob_id,
                response_tx,
            } => handle_get_share_indexes(backend, blob_id, response_tx).await,
            Self::GetBlobLightShares {
                blob_id,
                response_tx,
            } => handle_get_blob_light_shares(backend, blob_id, response_tx).await,
        }
    }
}

async fn handle_get_shared_commitments<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<<B as StorageDaApi>::Commitments>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_shared_commitments(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get shared commitments request".to_owned(),
        });
    }
    Ok(())
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

async fn handle_get_blob_light_shares<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<Vec<<B as StorageDaApi>::Share>>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_blob_light_shares(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get blob light shares request".to_owned(),
        });
    }
    Ok(())
}

async fn handle_get_share_indexes<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<HashSet<<B as StorageDaApi>::ShareIndex>>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_blob_share_indices(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get light share indexes request".to_owned(),
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
    pub const fn get_blob_light_shares_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        response_tx: Sender<Option<Vec<<Api as StorageDaApi>::Share>>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetBlobLightShares {
                blob_id,
                response_tx,
            }),
        }
    }

    #[must_use]
    pub const fn get_light_share_indexes_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        response_tx: Sender<Option<HashSet<<Api as StorageDaApi>::ShareIndex>>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetLightShareIndexes {
                blob_id,
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
    pub const fn get_shared_commitments_request(
        blob_id: <Api as StorageDaApi>::BlobId,
        response_tx: Sender<Option<<Api as StorageDaApi>::Commitments>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetSharedCommitments {
                blob_id,
                response_tx,
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
