use crate::{
    api::StorageBackendApi,
    backends::{rocksdb::RocksBackend, StorageSerde},
};

pub mod blocks;
pub mod da;
pub mod utils;

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for RocksBackend<SerdeOp> {}
