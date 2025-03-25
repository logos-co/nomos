pub mod backends;

use std::{
    fmt::{Debug, Display, Formatter},
    marker::PhantomData,
};

use async_trait::async_trait;
use backends::{StorageBackend, StorageSerde, StorageTransaction};
use bytes::Bytes;
use futures::StreamExt;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use serde::{de::DeserializeOwned, Serialize};
use services_utils::overwatch::lifecycle;
use tracing::error;

/// Storage message that maps to [`StorageBackend`] trait
pub enum StorageMsg<Backend: StorageBackend> {
    Load {
        key: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Option<Bytes>>,
    },
    LoadPrefix {
        prefix: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Vec<Bytes>>,
    },
    Store {
        key: Bytes,
        value: Bytes,
    },
    Remove {
        key: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Option<Bytes>>,
    },
    Execute {
        transaction: Backend::Transaction,
        reply_channel:
            tokio::sync::oneshot::Sender<<Backend::Transaction as StorageTransaction>::Result>,
    },
}

/// Reply channel for storage messages
pub struct StorageReplyReceiver<T, Backend> {
    channel: tokio::sync::oneshot::Receiver<T>,
    _backend: PhantomData<Backend>,
}

impl<T, Backend> StorageReplyReceiver<T, Backend> {
    #[must_use]
    pub const fn new(channel: tokio::sync::oneshot::Receiver<T>) -> Self {
        Self {
            channel,
            _backend: PhantomData,
        }
    }

    #[must_use]
    pub fn into_inner(self) -> tokio::sync::oneshot::Receiver<T> {
        self.channel
    }
}

impl<Backend: StorageBackend> StorageReplyReceiver<Option<Bytes>, Backend> {
    /// Receive and transform the reply into the desired type
    /// Target type must implement `From` from the original backend stored type.
    pub async fn recv<Output>(
        self,
    ) -> Result<Option<Output>, tokio::sync::oneshot::error::RecvError>
    where
        Output: DeserializeOwned,
    {
        self.channel
            .await
            // TODO: This should probably just return a result anyway. But for now we can consider
            // in infallible.
            .map(|maybe_bytes| {
                maybe_bytes.map(|bytes| {
                    Backend::SerdeOperator::deserialize(bytes)
                        .expect("Recovery from storage should never fail")
                })
            })
    }
}

impl<Backend: StorageBackend> StorageMsg<Backend> {
    pub fn new_load_message<K: Serialize>(
        key: K,
    ) -> (Self, StorageReplyReceiver<Option<Bytes>, Backend>) {
        let key = Backend::SerdeOperator::serialize(key);
        let (reply_channel, receiver) = tokio::sync::oneshot::channel();
        (
            Self::Load { key, reply_channel },
            StorageReplyReceiver::new(receiver),
        )
    }

    pub fn new_store_message<K: Serialize, V: Serialize>(key: K, value: V) -> Self {
        let key = Backend::SerdeOperator::serialize(key);
        let value = Backend::SerdeOperator::serialize(value);
        Self::Store { key, value }
    }

    pub fn new_remove_message<K: Serialize>(
        key: K,
    ) -> (Self, StorageReplyReceiver<Option<Bytes>, Backend>) {
        let key = Backend::SerdeOperator::serialize(key);
        let (reply_channel, receiver) = tokio::sync::oneshot::channel();
        (
            Self::Remove { key, reply_channel },
            StorageReplyReceiver::new(receiver),
        )
    }

    pub fn new_transaction_message(
        transaction: Backend::Transaction,
    ) -> (
        Self,
        StorageReplyReceiver<<Backend::Transaction as StorageTransaction>::Result, Backend>,
    ) {
        let (reply_channel, receiver) = tokio::sync::oneshot::channel();
        (
            Self::Execute {
                transaction,
                reply_channel,
            },
            StorageReplyReceiver::new(receiver),
        )
    }
}

// Implement `Debug` manually to avoid constraining `Backend` to `Debug`
impl<Backend: StorageBackend> Debug for StorageMsg<Backend> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load { key, .. } => {
                write!(f, "Load {{ {key:?} }}")
            }
            Self::LoadPrefix { prefix, .. } => {
                write!(f, "LoadPrefix {{ {prefix:?} }}")
            }
            Self::Store { key, value } => {
                write!(f, "Store {{ {key:?}, {value:?}}}")
            }
            Self::Remove { key, .. } => {
                write!(f, "Remove {{ {key:?} }}")
            }
            Self::Execute { .. } => write!(f, "Execute transaction"),
        }
    }
}

/// Storage error
/// Errors that may happen when performing storage operations
#[derive(Debug, thiserror::Error)]
enum StorageServiceError<Backend: StorageBackend> {
    #[error("Couldn't send a reply for operation `{operation}` with key [{key:?}]")]
    ReplyError { operation: String, key: Bytes },
    #[error("Storage backend error")]
    BackendError(#[source] Backend::Error),
}

/// Storage service that wraps a [`StorageBackend`]
pub struct StorageService<Backend: StorageBackend + Send + Sync + 'static, RuntimeServiceId> {
    backend: Backend,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<Backend: StorageBackend + Send + Sync + 'static, RuntimeServiceId>
    StorageService<Backend, RuntimeServiceId>
{
    async fn handle_storage_message(msg: StorageMsg<Backend>, backend: &mut Backend) {
        if let Err(e) = match msg {
            StorageMsg::Load { key, reply_channel } => {
                Self::handle_load(backend, key, reply_channel).await
            }
            StorageMsg::LoadPrefix {
                prefix,
                reply_channel,
            } => Self::handle_load_prefix(backend, prefix, reply_channel).await,
            StorageMsg::Store { key, value } => Self::handle_store(backend, key, value).await,
            StorageMsg::Remove { key, reply_channel } => {
                Self::handle_remove(backend, key, reply_channel).await
            }
            StorageMsg::Execute {
                transaction,
                reply_channel,
            } => Self::handle_execute(backend, transaction, reply_channel).await,
        } {
            // TODO: add proper logging
            println!("{e}");
        }
    }
    /// Handle load message
    async fn handle_load(
        backend: &mut Backend,
        key: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Option<Bytes>>,
    ) -> Result<(), StorageServiceError<Backend>> {
        let result: Option<Bytes> = backend
            .load(&key)
            .await
            .map_err(StorageServiceError::BackendError)?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                operation: "Load".to_owned(),
                key,
            })
    }

    /// Handle load prefix message
    async fn handle_load_prefix(
        backend: &mut Backend,
        prefix: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Vec<Bytes>>,
    ) -> Result<(), StorageServiceError<Backend>> {
        let result: Vec<Bytes> = backend
            .load_prefix(&prefix)
            .await
            .map_err(StorageServiceError::BackendError)?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                operation: "LoadPrefix".to_owned(),
                key: prefix,
            })
    }

    /// Handle remove message
    async fn handle_remove(
        backend: &mut Backend,
        key: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Option<Bytes>>,
    ) -> Result<(), StorageServiceError<Backend>> {
        let result: Option<Bytes> = backend
            .remove(&key)
            .await
            .map_err(StorageServiceError::BackendError)?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                operation: "Remove".to_owned(),
                key,
            })
    }

    /// Handle store message
    async fn handle_store(
        backend: &mut Backend,
        key: Bytes,
        value: Bytes,
    ) -> Result<(), StorageServiceError<Backend>> {
        backend
            .store(key, value)
            .await
            .map_err(StorageServiceError::BackendError)
    }

    /// Handle execute message
    async fn handle_execute(
        backend: &mut Backend,
        transaction: Backend::Transaction,
        reply_channel: tokio::sync::oneshot::Sender<
            <Backend::Transaction as StorageTransaction>::Result,
        >,
    ) -> Result<(), StorageServiceError<Backend>> {
        let result = backend
            .execute(transaction)
            .await
            .map_err(StorageServiceError::BackendError)?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                operation: "Execute".to_owned(),
                key: Bytes::new(),
            })
    }
}

#[async_trait]
impl<Backend: StorageBackend + Send + Sync + 'static, RuntimeServiceId>
    ServiceCore<RuntimeServiceId> for StorageService<Backend, RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self> + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: Backend::new(service_state.settings_reader.get_updated_settings())?,
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            mut backend,
            service_state:
                OpaqueServiceStateHandle::<Self, RuntimeServiceId> {
                    mut inbound_relay,
                    lifecycle_handle,
                    ..
                },
        } = self;
        let mut lifecycle_stream = lifecycle_handle.message_stream();
        let backend = &mut backend;
        #[expect(
            clippy::redundant_pub_crate,
            reason = "Generated by `tokio::select` macro."
        )]
        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_storage_message(msg, backend).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        // TODO: Try to finish pending transactions if any and close connections properly
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<Backend: StorageBackend + Send + Sync, RuntimeServiceId> ServiceData
    for StorageService<Backend, RuntimeServiceId>
{
    type Settings = Backend::Settings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = StorageMsg<Backend>;
}
