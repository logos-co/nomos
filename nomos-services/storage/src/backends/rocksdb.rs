use std::{marker::PhantomData, path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
pub use rocksdb::Error;
use rocksdb::{Options, DB};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::timeout;

use super::{StorageBackend, StorageSerde, StorageTransaction};

const TIMEOUT: Duration = Duration::from_secs(32);

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("Background task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),

    #[error("Storage operation timed out")]
    Timeout,
}
/// Rocks backend setting
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RocksBackendSettings {
    /// File path to the db file
    pub db_path: PathBuf,
    pub read_only: bool,
    pub column_family: Option<String>,
}

/// Rocks transaction type
// Do not use `TransactionDB` here, because rocksdb's `TransactionDB` does not
// support open by read-only mode. Thus, we cannot open the same db in two or
// more processes.
pub struct Transaction {
    rocks: Arc<DB>,
    #[expect(clippy::type_complexity, reason = "TODO: Address this at some point.")]
    executor: Box<dyn FnOnce(&DB) -> Result<Option<Bytes>, Error> + Send + Sync>,
}

impl Transaction {
    /// Execute a function over the transaction
    pub fn execute(self) -> Result<Option<Bytes>, Error> {
        (self.executor)(&self.rocks)
    }
}

impl StorageTransaction for Transaction {
    type Result = Result<Option<Bytes>, Error>;
    type Transaction = Self;
}

/// Rocks storage backend
pub struct RocksBackend<SerdeOp> {
    rocks: Arc<DB>,
    _serde_op: PhantomData<SerdeOp>,
}

impl<SerdeOp> RocksBackend<SerdeOp> {
    pub fn txn(
        &self,
        executor: impl FnOnce(&DB) -> Result<Option<Bytes>, Error> + Send + Sync + 'static,
    ) -> Transaction {
        Transaction {
            rocks: Arc::clone(&self.rocks),
            executor: Box::new(executor),
        }
    }
}

impl<SerdeOp> core::fmt::Debug for RocksBackend<SerdeOp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format!("RocksBackend {{ rocks: {:?} }}", self.rocks).fmt(f)
    }
}

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageBackend for RocksBackend<SerdeOp> {
    type Settings = RocksBackendSettings;
    type Error = StorageError;
    type Transaction = Transaction;
    type SerdeOperator = SerdeOp;

    fn new(config: Self::Settings) -> Result<Self, Self::Error> {
        let RocksBackendSettings {
            db_path,
            read_only,
            column_family: cf,
        } = config;

        let db = match (read_only, cf) {
            (true, None) => {
                let mut opts = Options::default();
                opts.create_if_missing(false);
                DB::open_for_read_only(&opts, db_path, false)?
            }
            (true, Some(cf)) => {
                let mut opts = Options::default();
                opts.create_if_missing(false);
                DB::open_cf_for_read_only(&opts, db_path, [cf], false)?
            }
            (false, None) => {
                let mut opts = Options::default();
                opts.create_if_missing(true);
                opts.create_missing_column_families(true);
                DB::open(&opts, db_path)?
            }
            (false, Some(cf)) => {
                let mut opts = Options::default();
                opts.create_if_missing(true);
                opts.create_missing_column_families(true);
                DB::open_cf(&opts, db_path, [cf])?
            }
        };

        Ok(Self {
            rocks: Arc::new(db),
            _serde_op: PhantomData,
        })
    }

    async fn store(&mut self, key: Bytes, value: Bytes) -> Result<(), Self::Error> {
        let rocks = Arc::clone(&self.rocks);
        let _ = timeout(
            TIMEOUT,
            tokio::task::spawn_blocking(move || rocks.put(key, value)),
        )
        .await
        .map_err(|_| StorageError::Timeout)??;
        Ok(())
    }

    async fn load(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        let rocks = Arc::clone(&self.rocks);
        let key = key.to_vec();

        let result = timeout(
            TIMEOUT,
            tokio::task::spawn_blocking(move || rocks.get(&key).map(|opt| opt.map(Bytes::from))),
        )
        .await
        .map_err(|_| StorageError::Timeout)??;

        Ok(result?)
    }

    async fn load_prefix(&mut self, prefix: &[u8]) -> Result<Vec<Bytes>, Self::Error> {
        let rocks = Arc::clone(&self.rocks);
        let prefix = prefix.to_vec();

        let values = timeout(
            TIMEOUT,
            tokio::task::spawn_blocking(move || {
                let mut values = Vec::new();
                for item in rocks.prefix_iterator(&prefix) {
                    match item {
                        Ok((_key, value)) => values.push(Bytes::from(value.to_vec())),
                        Err(e) => return Err(e),
                    }
                }
                Ok(values)
            }),
        )
        .await
        .map_err(|_| StorageError::Timeout)??;

        Ok(values?)
    }

    async fn remove(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        let val = self.load(key).await?;

        if val.is_some() {
            let rocks = Arc::clone(&self.rocks);
            let key = key.to_vec();

            let _ = timeout(
                TIMEOUT,
                tokio::task::spawn_blocking(move || rocks.delete(&key)),
            )
            .await
            .map_err(|_| StorageError::Timeout)??;

            Ok(val)
        } else {
            Ok(None)
        }
    }

    async fn execute(
        &mut self,
        transaction: Self::Transaction,
    ) -> Result<<Self::Transaction as StorageTransaction>::Result, Self::Error> {
        let result = timeout(
            TIMEOUT,
            tokio::task::spawn_blocking(move || transaction.execute()),
        )
        .await
        .map_err(|_| StorageError::Timeout)??;
        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;

    use super::{super::testing::NoStorageSerde, *};

    #[tokio::test]
    async fn test_store_load_remove(
    ) -> Result<(), <RocksBackend<NoStorageSerde> as StorageBackend>::Error> {
        let temp_path = TempDir::new().unwrap();
        let sled_settings = RocksBackendSettings {
            db_path: temp_path.path().to_path_buf(),
            read_only: false,
            column_family: None,
        };
        let key = "foo";
        let value = "bar";

        let mut db: RocksBackend<NoStorageSerde> = RocksBackend::new(sled_settings)?;
        db.store(key.as_bytes().into(), value.as_bytes().into())
            .await?;
        let load_value = db.load(key.as_bytes()).await?;
        assert_eq!(load_value, Some(value.as_bytes().into()));
        let removed_value = db.remove(key.as_bytes()).await?;
        assert_eq!(removed_value, Some(value.as_bytes().into()));

        Ok(())
    }

    #[tokio::test]
    async fn test_transaction(
    ) -> Result<(), <RocksBackend<NoStorageSerde> as StorageBackend>::Error> {
        let temp_path = TempDir::new().unwrap();

        let sled_settings = RocksBackendSettings {
            db_path: temp_path.path().to_path_buf(),
            read_only: false,
            column_family: None,
        };

        let mut db: RocksBackend<NoStorageSerde> = RocksBackend::new(sled_settings)?;
        let txn = db.txn(|db| {
            let key = "foo";
            let value = "bar";
            db.put(key, value)?;
            let result = db.get(key)?;
            db.delete(key)?;
            Ok(result.map(std::convert::Into::into))
        });
        let result = db.execute(txn).await??;
        assert_eq!(result, Some(b"bar".as_ref().into()));

        Ok(())
    }

    #[tokio::test]
    async fn test_multi_readers_single_writer(
    ) -> Result<(), <RocksBackend<NoStorageSerde> as StorageBackend>::Error> {
        use tokio::sync::mpsc::channel;

        let temp_path = TempDir::new().unwrap();
        let path = temp_path.path().to_path_buf();
        let sled_settings = RocksBackendSettings {
            db_path: temp_path.path().to_path_buf(),
            read_only: false,
            column_family: None,
        };
        let key = "foo";
        let value = "bar";

        let mut db: RocksBackend<NoStorageSerde> = RocksBackend::new(sled_settings)?;

        let (tx, mut rx) = channel(5);
        // now let us spawn a few readers
        for _ in 0..5 {
            let p = path.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async move {
                        let sled_settings = RocksBackendSettings {
                            db_path: p,
                            read_only: true,
                            column_family: None,
                        };
                        let key = "foo";

                        let mut db: RocksBackend<NoStorageSerde> =
                            RocksBackend::new(sled_settings).unwrap();

                        while db.load(key.as_bytes()).await.unwrap().is_none() {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }

                        tx.send(()).await.unwrap();
                    });
            });
        }

        db.store(key.as_bytes().into(), value.as_bytes().into())
            .await?;

        let mut recvs = 0;
        loop {
            if rx.recv().await.is_some() {
                recvs += 1;
                if recvs == 5 {
                    break;
                }
            }
        }
        Ok(())
    }
}
