use crate::{
    api::{StorageBackendApi, StorageFunctions},
    backends::{rocksdb::RocksBackend, StorageSerde},
};

pub mod rocksdb;

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageFunctions for RocksBackend<SerdeOp> {}

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for RocksBackend<SerdeOp> {}
