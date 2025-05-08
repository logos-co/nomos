use std::fmt::Debug;

use async_trait::async_trait;
use overwatch::DynError;
use requests::DaApiRequest;
use tracing::error;

use crate::{StorageServiceError, api::StorageBackendApi};

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

pub async fn handle_da_api_call<Backend: StorageBackendApi>(
    method: DaApiRequest<
        <Backend as StorageDaApi>::BlobId,
        <Backend as StorageDaApi>::Share,
        <Backend as StorageDaApi>::Commitments,
        <Backend as StorageDaApi>::ShareIndex,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match method {
        DaApiRequest::GetLightShare { .. } => handle_get_light_share(method, api_backend).await,
        DaApiRequest::StoreLightShare { .. } => handle_store_light_share(method, api_backend).await,
        DaApiRequest::StoreSharedCommitments { .. } => {
            handle_store_shared_commitments(method, api_backend).await
        }
    }
}

async fn handle_get_light_share<Backend: StorageBackendApi>(
    request: DaApiRequest<
        <Backend as StorageDaApi>::BlobId,
        <Backend as StorageDaApi>::Share,
        <Backend as StorageDaApi>::Commitments,
        <Backend as StorageDaApi>::ShareIndex,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match request {
        DaApiRequest::GetLightShare {
            blob_id,
            share_idx,
            response_tx,
        } => {
            let result = api_backend
                .get_light_share(blob_id, share_idx)
                .await
                .map_err(|e| {
                    error!(
                        "Failed to get light share: {:?} with error: {:?}",
                        blob_id, e
                    );

                    let key = format!("{}:{}", "blob_id", "share_idx").into();
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
        _ => {
            unreachable!("invalid request")
        }
    }
}

async fn handle_store_light_share<Backend: StorageBackendApi>(
    request: DaApiRequest<
        <Backend as StorageDaApi>::BlobId,
        <Backend as StorageDaApi>::Share,
        <Backend as StorageDaApi>::Commitments,
        <Backend as StorageDaApi>::ShareIndex,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match request {
        DaApiRequest::StoreLightShare {
            blob_id,
            share_idx,
            light_share,
        } => {
            api_backend
                .store_light_share(blob_id, share_idx, light_share)
                .await
                .map_err(|e| {
                    error!(
                        "Failed to store light share: {:?} with error: {:?}",
                        blob_id, e
                    );

                    let key = format!("{}:{}", "blob_id", "share_idx").into();
                    StorageServiceError::ReplyError {
                        operation: "store_light_share".to_owned(),
                        key,
                    }
                })?;

            Ok(())
        }
        _ => unreachable!("invalid request"),
    }
}

async fn handle_store_shared_commitments<Backend: StorageBackendApi>(
    request: DaApiRequest<
        <Backend as StorageDaApi>::BlobId,
        <Backend as StorageDaApi>::Share,
        <Backend as StorageDaApi>::Commitments,
        <Backend as StorageDaApi>::ShareIndex,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match request {
        DaApiRequest::StoreSharedCommitments {
            blob_id,
            shared_commitments,
        } => {
            api_backend
                .store_shared_commitments(blob_id, shared_commitments)
                .await
                .map_err(|e| {
                    error!(
                        "Failed to store shared commitments: {:?} with error: {:?}",
                        blob_id, e
                    );

                    let key = "blob_id".to_owned().into();
                    StorageServiceError::ReplyError {
                        operation: "store_shared_commitments".to_owned(),
                        key,
                    }
                })?;

            Ok(())
        }
        _ => unreachable!("invalid request"),
    }
}
