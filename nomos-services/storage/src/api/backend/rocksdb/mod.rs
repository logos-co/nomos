use bytes::Bytes;

use crate::{
    api::StorageBackendApi,
    backends::{rocksdb::RocksBackend, StorageSerde},
};

pub mod blocks;
pub mod da;
pub mod utils;

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for RocksBackend<SerdeOp> {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("RocksDB error: {0}")]
    RocksDbError(#[from] rocksdb::Error),
    #[error("Inconsistent value format: {0:?}")]
    InconsistentValueFormat(Bytes),
}
