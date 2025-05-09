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
pub trait StorageBackendApi: StorageFunctions + StorageBackend
where
    Self: StorageChainApi<HeaderId = HeaderId, Block = Bytes>
        + StorageDaApi<BlobId = [u8; 32], Share = Bytes, Commitments = Bytes, ShareIndex = [u8; 2]>,
{
}

pub(crate) trait Executable<B: StorageBackendApi> {
    async fn execute(self, api: &mut B) -> Result<(), StorageServiceError<B>>;
}

pub enum StorageApiRequest<Api: StorageBackendApi> {
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

impl<B: StorageBackendApi> Executable<B> for StorageApiRequest<B>
where
    B: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError<B>> {
        match self {
            StorageApiRequest::Chain(request) => request.execute(backend).await,
            StorageApiRequest::Da(request) => request.execute(backend).await,
        }
    }
}
