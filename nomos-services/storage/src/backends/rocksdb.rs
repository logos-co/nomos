use std::{marker::PhantomData, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
pub use rocksdb::Error;
use rocksdb::{Options, DB};
use serde::{Deserialize, Serialize};

use super::{StorageBackend, StorageIterator, StorageSerde, StorageTransaction};

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
    type Error = rocksdb::Error;
    type Transaction = Transaction;
    type Iterator = RocksIterator;
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
        self.rocks.put(key, value)
    }

    async fn load(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        self.rocks
            .get(key)
            .map(|opt| opt.map(std::convert::Into::into))
    }

    async fn load_prefix(&mut self, prefix: &[u8]) -> Result<Vec<Bytes>, Self::Error> {
        let mut values = Vec::new();
        let iter = self.rocks.prefix_iterator(prefix);

        for item in iter {
            match item {
                Ok((_key, value)) => {
                    values.push(Bytes::from(value.to_vec()));
                }
                Err(e) => return Err(e), // Return the error if one occurs
            }
        }

        Ok(values)
    }

    async fn scan_range(&self, prefix: &[u8], start: &[u8]) -> Result<Self::Iterator, Self::Error> {
        // Returns just an wrapper of Arc<DB> instead of a real iterator,
        // because of the design of rocksdb API and borrow checker.
        // An iterator can be created by the returned [`RocksIterator`].
        Ok(RocksIterator::new(
            Arc::clone(&self.rocks),
            prefix.to_vec(),
            start.to_vec(),
        ))
    }

    async fn remove(&mut self, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        self.load(key).await.and_then(|val| {
            if val.is_some() {
                self.rocks.delete(key).map(|()| val)
            } else {
                Ok(None)
            }
        })
    }

    async fn execute(
        &mut self,
        transaction: Self::Transaction,
    ) -> Result<<Self::Transaction as StorageTransaction>::Result, Self::Error> {
        Ok(transaction.execute())
    }
}

pub struct RocksIterator {
    rocks: Arc<DB>,
    prefix: Vec<u8>,
    start: Vec<u8>,
}

impl RocksIterator {
    #[must_use]
    const fn new(rocks: Arc<DB>, prefix: Vec<u8>, start: Vec<u8>) -> Self {
        Self {
            rocks,
            prefix,
            start,
        }
    }
}

impl StorageIterator for RocksIterator {
    type Error = rocksdb::Error;

    /// Open a stream of values in the range of the iterator.
    /// The stream operate on a consistent snapshot of the DB
    /// taken at the moment the stream is created.
    /// Any updates made after the stream is created won't be visible
    /// to that stream.
    #[must_use]
    fn stream(&self) -> BoxStream<'_, Result<(Bytes, Bytes), Error>> {
        let iter = self.rocks.snapshot().iterator(rocksdb::IteratorMode::From(
            &[self.prefix.as_slice(), self.start.as_slice()].concat(),
            rocksdb::Direction::Forward,
        ));
        let stream = futures::stream::iter(
            iter.take_while(
                move |result| matches!(result, Ok((key, _)) if key.starts_with(&self.prefix)),
            )
            .map(move |result| match result {
                Ok((key, value)) => Ok((Bytes::from(key.to_vec()), Bytes::from(value.to_vec()))),
                Err(e) => Err(e),
            }),
        );
        Box::pin(stream)
    }
}

#[cfg(test)]
mod test {
    use futures::StreamExt;
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

    #[tokio::test]
    async fn test_scan_range() {
        let temp_path = TempDir::new().unwrap();
        let settings = RocksBackendSettings {
            db_path: temp_path.path().to_path_buf(),
            read_only: false,
            column_family: None,
        };

        let mut db: RocksBackend<NoStorageSerde> = RocksBackend::new(settings).unwrap();

        // Store test data with different prefixes
        db.store(b"prefix1/a".to_vec().into(), b"v1".to_vec().into())
            .await
            .unwrap();
        db.store(b"prefix1/b".to_vec().into(), b"v2".to_vec().into())
            .await
            .unwrap();
        db.store(b"prefix1/c".to_vec().into(), b"v3".to_vec().into())
            .await
            .unwrap();
        db.store(b"prefix2/a".to_vec().into(), b"v4".to_vec().into())
            .await
            .unwrap();
        db.store(b"prefix2/b".to_vec().into(), b"v5".to_vec().into())
            .await
            .unwrap();

        // Test 1: Basic range scan within prefix
        let iterator = db.scan_range(b"prefix1/", b"b").await.unwrap();
        let mut stream = iterator.stream();
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix1/b".to_vec().into(), b"v2".to_vec().into())))
        );
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix1/c".to_vec().into(), b"v3".to_vec().into())))
        );
        assert_eq!(stream.next().await, None);

        // Test 2: Snapshot isolation
        let iterator = db.scan_range(b"prefix1/", b"b").await.unwrap();
        let mut stream = iterator.stream();
        // Add new data after creating stream. This should not appear in the stream.
        db.store(b"prefix1/bb".to_vec().into(), b"v6".to_vec().into())
            .await
            .unwrap();
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix1/b".to_vec().into(), b"v2".to_vec().into())))
        );
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix1/c".to_vec().into(), b"v3".to_vec().into())))
        );
        assert_eq!(stream.next().await, None);

        // Test 3: Empty range
        let iterator = db.scan_range(b"prefix3/", b"a").await.unwrap();
        let mut stream = iterator.stream();
        assert_eq!(stream.next().await, None);

        // Test 4: Start from the prefix that is not the first prefix in the DB
        let iterator = db.scan_range(b"prefix2/", b"").await.unwrap();
        let mut stream = iterator.stream();
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix2/a".to_vec().into(), b"v4".to_vec().into())))
        );
        assert_eq!(
            stream.next().await,
            Some(Ok((b"prefix2/b".to_vec().into(), b"v5".to_vec().into())))
        );
        assert_eq!(stream.next().await, None);

        // Test 5: Start position beyond available data
        let iterator = db.scan_range(b"prefix1/", b"z").await.unwrap();
        let mut stream = iterator.stream();
        assert_eq!(stream.next().await, None);
    }
}
