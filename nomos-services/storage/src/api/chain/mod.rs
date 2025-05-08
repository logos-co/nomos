pub mod requests;

use std::fmt::Debug;

use async_trait::async_trait;
use overwatch::DynError;
use requests::ChainApiRequest;
use tracing::error;

use crate::{StorageServiceError, api::StorageBackendApi};

#[async_trait]
pub trait StorageChainApi {
    type HeaderId;
    type Block: Debug;

    async fn get_block(
        &mut self,
        header_id: Self::HeaderId,
    ) -> Result<Option<Self::Block>, DynError>;

    async fn store_block(
        &mut self,
        header_id: Self::HeaderId,
        block: Self::Block,
    ) -> Result<(), DynError>;
}

pub async fn handle_chain_api_call<Backend: StorageBackendApi>(
    method: ChainApiRequest<
        <Backend as StorageChainApi>::HeaderId,
        <Backend as StorageChainApi>::Block,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match method {
        ChainApiRequest::GetBlock { .. } => handle_get_block(method, api_backend).await,
        ChainApiRequest::StoreBlock { .. } => handle_store_block(method, api_backend).await,
    }
}

async fn handle_get_block<Backend: StorageBackendApi>(
    request: ChainApiRequest<
        <Backend as StorageChainApi>::HeaderId,
        <Backend as StorageChainApi>::Block,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match request {
        ChainApiRequest::GetBlock {
            header_id,
            response_tx,
        } => {
            let result = api_backend.get_block(header_id).await.map_err(|e| {
                error!("Failed to get block: {:?}", e);

                let key = "header_id".to_owned().into();
                StorageServiceError::ReplyError {
                    operation: "get_block".to_owned(),
                    key,
                }
            })?;

            if response_tx.send(result).is_err() {
                error!("Failed to send response in get_block");
            }

            Ok(())
        }
        ChainApiRequest::StoreBlock { .. } => {
            unreachable!("handle_get_block should only be called with GetBlock request")
        }
    }
}

async fn handle_store_block<Backend: StorageBackendApi>(
    request: ChainApiRequest<
        <Backend as StorageChainApi>::HeaderId,
        <Backend as StorageChainApi>::Block,
    >,
    api_backend: &mut Backend,
) -> Result<(), StorageServiceError<Backend>> {
    match request {
        ChainApiRequest::StoreBlock { header_id, block } => {
            api_backend
                .store_block(header_id, block)
                .await
                .map_err(|e| {
                    error!("Failed to store block: {:?}", e);

                    let key = "header_id".to_owned().into();
                    StorageServiceError::ReplyError {
                        operation: "store_block".to_owned(),
                        key,
                    }
                })?;

            Ok(())
        }
        ChainApiRequest::GetBlock { .. } => {
            unreachable!("handle_store_block should only be called with StoreBlock request")
        }
    }
}
