use async_trait::async_trait;
use bytes::Bytes;
use nomos_core::header::HeaderId;

use crate::{
    api::{
        chain::{StorageChainApi, requests::ChainApiRequest},
        da::{StorageDaApi, requests::DaApiRequest},
    },
    backends::StorageBackend,
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
