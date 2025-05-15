use std::{collections::HashSet, fmt::Display};

use nomos_core::header::HeaderId;
use tokio::sync::oneshot::Sender;

use crate::{
    api::{chain::StorageChainApi, StorageApiRequest, StorageBackendApi, StorageOperation},
    backends::StorageBackend,
    StorageMsg, StorageServiceError,
};

pub enum ChainApiRequest<Backend: StorageBackend> {
    GetBlock {
        header_id: HeaderId,
        response_tx: Sender<Option<<Backend as StorageChainApi>::Block>>,
    },
    StoreBlock {
        header_id: HeaderId,
        block: <Backend as StorageChainApi>::Block,
    },
    RemoveBlock {
        header_id: HeaderId,
        response_tx: Sender<Result<B::Block, StorageServiceError>>,
    },
    RemoveBlocks {
        header_ids: HashSet<HeaderId>,
        response_tx: Sender<Result<Vec<B::Block>, StorageServiceError>>,
    },
}

impl<Backend> StorageOperation<Backend> for ChainApiRequest<Backend>
where
    Backend: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut Backend) -> Result<(), StorageServiceError> {
        match self {
            Self::GetBlock {
                header_id,
                response_tx,
            } => handle_get_block(backend, header_id, response_tx).await,
            Self::StoreBlock { header_id, block } => {
                handle_store_block(backend, header_id, block).await
            }
            Self::RemoveBlock {
                header_id,
                response_tx,
            } => handle_remove_block(backend, header_id, response_tx).await,
            Self::RemoveBlocks {
                header_ids,
                response_tx,
            } => handle_remove_blocks(backend, header_ids.into_iter(), response_tx).await,
        }
    }
}

async fn handle_get_block<Backend: StorageBackend>(
    backend: &mut Backend,
    header_id: HeaderId,
    response_tx: Sender<Option<Backend::Block>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_block(header_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: format!(
                "Failed to send reply for get block request by header_id: {header_id}"
            ),
        });
    }

    Ok(())
}

async fn handle_store_block<Backend: StorageBackend>(
    backend: &mut Backend,
    header_id: HeaderId,
    block: Backend::Block,
) -> Result<(), StorageServiceError> {
    backend
        .store_block(header_id, block)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_remove_block<B>(
    backend: &mut B,
    header_id: HeaderId,
    response_tx: Sender<Result<B::Block, StorageServiceError>>,
) -> Result<(), StorageServiceError>
where
    B: StorageBackend<Error: Display>,
{
    let result = backend
        .remove_block(header_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()));
    if let Err(Err(e)) = response_tx.send(result) {
        return Err(StorageServiceError::ReplyError {
            message: format!("Backend error: {e}"),
        });
    }

    Ok(())
}

async fn handle_remove_blocks<B, Headers>(
    backend: &mut B,
    header_ids: Headers,
    response_tx: Sender<Result<Vec<B::Block>, StorageServiceError>>,
) -> Result<(), StorageServiceError>
where
    B: StorageBackend<Error: Display>,
    Headers: Iterator<Item = HeaderId>,
{
    let result = backend
        .remove_blocks(header_ids)
        .await
        .map(|blocks| blocks.into_iter().collect())
        .map_err(|e| StorageServiceError::BackendError(e.into()));
    if let Err(Err(e)) = response_tx.send(result) {
        return Err(StorageServiceError::ReplyError {
            message: format!("Backend error: {e}"),
        });
    }

    Ok(())
}

impl<Api: StorageBackend> StorageMsg<Api> {
    #[must_use]
    pub const fn get_block_request(
        header_id: HeaderId,
        response_tx: Sender<Option<<Api as StorageChainApi>::Block>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::GetBlock {
                header_id,
                response_tx,
            }),
        }
    }

    pub const fn store_block_request(
        header_id: HeaderId,
        block: <Api as StorageChainApi>::Block,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::StoreBlock { header_id, block }),
        }
    }

    #[must_use]
    pub const fn remove_block_request(
        header_id: HeaderId,
        response_tx: Sender<Result<Api::Block, StorageServiceError>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::RemoveBlock {
                header_id,
                response_tx,
            }),
        }
    }

    #[must_use]
    pub const fn remove_blocks_request(
        header_ids: HashSet<HeaderId>,
        response_tx: Sender<Result<Vec<Api::Block>, StorageServiceError>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::RemoveBlocks {
                header_ids,
                response_tx,
            }),
        }
    }
}
