use tokio::sync::oneshot::Sender;
use tracing::error;

use crate::{
    api::{da::StorageDaApi, Executable, StorageApiRequest, StorageBackendApi},
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

impl<B> Executable<B> for DaApiRequest<B>
where
    B: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError<B>> {
        match self {
            DaApiRequest::GetLightShare {
                blob_id,
                share_idx,
                response_tx,
            } => handle_get_light_share(backend, blob_id, share_idx, response_tx).await,
            DaApiRequest::StoreLightShare {
                blob_id,
                share_idx,
                light_share,
            } => handle_store_light_share(backend, blob_id, share_idx, light_share).await,
            DaApiRequest::StoreSharedCommitments {
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
) -> Result<(), StorageServiceError<B>> {
    let result = backend
        .get_light_share(blob_id, share_idx)
        .await
        .map_err(|e| {
            error!("Failed to get light share: {:?}", e);

            let key = "blob_id".to_owned().into();
            StorageServiceError::ReplyError {
                operation: "get_light_share".to_owned(),
                key,
            }
        })?;

    if response_tx.send(result).is_err() {
        error!("Failed to send response in get_light_share");
    }

    Ok(())
}

async fn handle_store_light_share<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    share_idx: B::ShareIndex,
    light_share: B::Share,
) -> Result<(), StorageServiceError<B>> {
    backend
        .store_light_share(blob_id, share_idx, light_share)
        .await
        .map_err(|e| {
            error!("Failed to store light share: {:?}", e);

            let key = "blob_id".to_owned().into();
            StorageServiceError::ReplyError {
                operation: "store_light_share".to_owned(),
                key,
            }
        })?;
    Ok(())
}

async fn handle_store_shared_commitments<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    shared_commitments: B::Commitments,
) -> Result<(), StorageServiceError<B>> {
    backend
        .store_shared_commitments(blob_id, shared_commitments)
        .await
        .map_err(|e| {
            error!("Failed to store shared commitments: {:?}", e);

            let key = "blob_id".to_owned().into();
            StorageServiceError::ReplyError {
                operation: "store_shared_commitments".to_owned(),
                key,
            }
        })?;

    Ok(())
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
