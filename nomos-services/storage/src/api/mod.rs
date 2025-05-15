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

pub(crate) trait StorageOperation<B: StorageBackend> {
    async fn execute(self, api: &mut B) -> Result<(), StorageServiceError>;
}

pub enum StorageApiRequest<B: StorageBackend> {
    Chain(ChainApiRequest<B>),
    Da(DaApiRequest<B>),
}

impl<B: StorageBackend> StorageOperation<B> for StorageApiRequest<B> {
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError> {
        match self {
            Self::Chain(request) => request.execute(backend).await,
            Self::Da(request) => request.execute(backend).await,
        }
    }
}
