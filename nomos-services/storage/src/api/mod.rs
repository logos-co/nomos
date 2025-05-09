use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;

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
pub trait StorageFunctions: StorageChainApi + StorageDaApi {}

#[async_trait]
pub trait StorageBackendApi: StorageFunctions
where
    Self: StorageChainApi<HeaderId = HeaderId, Block = Bytes>
        + StorageDaApi<BlobId = [u8; 32], Share = Bytes, Commitments = Bytes, ShareIndex = [u8; 2]>,
{
}

pub(crate) trait Executable<B: StorageBackend> {
    async fn execute(self, api: &mut B) -> Result<(), StorageServiceError<B>>;
}

pub enum StorageApiRequest<Api: StorageBackend> {
    Chain(ChainApiRequest<<Api as StorageChainApi>::HeaderId, <Api as StorageChainApi>::Block>),
    Da(
        DaApiRequest<
            <Api as StorageDaApi>::BlobId,
            <Api as StorageDaApi>::Share,
            <Api as StorageDaApi>::Commitments,
            <Api as StorageDaApi>::ShareIndex,
        >,
    ),
}

impl<B: StorageBackend> Executable<B> for StorageApiRequest<B> {
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError<B>> {
        match self {
            Self::Chain(request) => request.execute(backend).await,
            Self::Da(request) => request.execute(backend).await,
        }
    }
}
