use crate::{
    api::{StorageBackendApi, StorageFunctions},
    backends::{StorageSerde, rocksdb::RocksBackend},
};

pub mod rocksdb;

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageFunctions for RocksBackend<SerdeOp> {}

impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackendApi for RocksBackend<SerdeOp> {}
