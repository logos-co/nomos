use async_trait::async_trait;

use crate::{
    api::{
        chain::{requests::ChainApiRequest, StorageChainApi},
        da::{requests::DaApiRequest, StorageDaApi},
    },
    backends::StorageBackend,
    StorageServiceError,
};

pub mod backend;
pub mod chain;
pub mod da;

#[async_trait]
pub trait StorageBackendApi: StorageChainApi + StorageDaApi {}

pub(crate) trait StorageOperation<Backend: StorageBackend> {
    async fn execute(self, api: &mut Backend) -> Result<(), StorageServiceError>;
}

pub enum StorageApiRequest<Backend: StorageBackend> {
    Chain(ChainApiRequest<Backend>),
    Da(DaApiRequest<Backend>),
}

impl<Backend: StorageBackend> StorageOperation<Backend> for StorageApiRequest<Backend> {
    async fn execute(self, backend: &mut Backend) -> Result<(), StorageServiceError> {
        match self {
            Self::Chain(request) => request.execute(backend).await,
            Self::Da(request) => request.execute(backend).await,
        }
    }
}
