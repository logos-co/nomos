use nomos_core::header::HeaderId;
use tokio::sync::oneshot::Sender;

use crate::{
    api::{chain::StorageChainApi, StorageApiRequest, StorageBackendApi, StorageOperation},
    backends::StorageBackend,
    StorageMsg, StorageServiceError,
};

pub enum ChainApiRequest<B: StorageBackend> {
    GetBlock {
        header_id: HeaderId,
        response_tx: Sender<Option<<B as StorageChainApi>::Block>>,
    },
    StoreBlock {
        header_id: HeaderId,
        block: <B as StorageChainApi>::Block,
    },
}

impl<B> StorageOperation<B> for ChainApiRequest<B>
where
    B: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError> {
        match self {
            Self::GetBlock {
                header_id,
                response_tx,
            } => handle_get_block(backend, header_id, response_tx).await,
            Self::StoreBlock { header_id, block } => {
                handle_store_block(backend, header_id, block).await
            }
        }
    }
}

async fn handle_get_block<B: StorageBackend>(
    backend: &mut B,
    header_id: HeaderId,
    response_tx: Sender<Option<B::Block>>,
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

async fn handle_store_block<B: StorageBackend>(
    backend: &mut B,
    header_id: HeaderId,
    block: B::Block,
) -> Result<(), StorageServiceError> {
    backend
        .store_block(header_id, block)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
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
}
