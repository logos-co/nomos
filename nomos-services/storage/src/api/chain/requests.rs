use crate::{
    api::{chain::StorageChainApi, StorageApiRequest, StorageBackendApi},
    backends::StorageBackend,
    StorageMsg,
};

pub enum ChainApiRequest<HeaderId, Block> {
    GetBlock {
        header_id: HeaderId,
        response_tx: tokio::sync::oneshot::Sender<Option<Block>>,
    },
    StoreBlock {
        header_id: HeaderId,
        block: Block,
    },
}

impl<Api: StorageBackend + StorageBackendApi> StorageMsg<Api> {
    #[must_use]
    pub const fn get_block_request(
        header_id: <Api as StorageChainApi>::HeaderId,
        response_tx: tokio::sync::oneshot::Sender<Option<<Api as StorageChainApi>::Block>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::GetBlock {
                header_id,
                response_tx,
            }),
        }
    }

    pub const fn store_block_request(
        header_id: <Api as StorageChainApi>::HeaderId,
        block: <Api as StorageChainApi>::Block,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Chain(ChainApiRequest::StoreBlock { header_id, block }),
        }
    }
}
