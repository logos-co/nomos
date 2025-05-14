pub mod api;
pub mod backends;

use std::{
    fmt::{Debug, Display, Formatter},
    marker::PhantomData,
};

use async_trait::async_trait;
use backends::{StorageBackend, StorageSerde as _, StorageTransaction};
use bytes::Bytes;
use futures::StreamExt as _;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{de::DeserializeOwned, Serialize};
use services_utils::overwatch::lifecycle;
use tracing::error;

use crate::api::{StorageApiRequest, StorageOperation};

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
    Api {
        request: StorageApiRequest<Backend>,
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
    pub fn new_load_message(key: Bytes) -> (Self, StorageReplyReceiver<Option<Bytes>, Backend>) {
        let (reply_channel, receiver) = tokio::sync::oneshot::channel();
        (
            Self::Load { key, reply_channel },
            StorageReplyReceiver::new(receiver),
        )
    }

    pub fn new_store_message<V: Serialize>(key: Bytes, value: V) -> Self {
        let value = Backend::SerdeOperator::serialize(value);
        Self::Store { key, value }
    }

    pub fn new_remove_message(key: Bytes) -> (Self, StorageReplyReceiver<Option<Bytes>, Backend>) {
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
            Self::Api { .. } => {
                write!(f, "Api {{ .. }}")
            }
        }
    }
}

/// Storage error
/// Errors that may happen when performing storage operations
#[derive(Debug, thiserror::Error)]
pub enum StorageServiceError {
    #[error("Couldn't send a reply [{message:?}]")]
    ReplyError { message: String },
    #[error("Storage backend error")]
    BackendError(Box<dyn std::error::Error + Send + Sync>),
}

/// Storage service that wraps a [`StorageBackend`]
pub struct StorageService<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
{
    backend: Backend,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<Backend, RuntimeServiceId> StorageService<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
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
            StorageMsg::Api { request: api_call } => Self::handle_api_call(api_call, backend).await,
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
    ) -> Result<(), StorageServiceError> {
        let result: Option<Bytes> = backend
            .load(&key)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                message: format!("Load {key:?}"),
            })
    }

    /// Handle load prefix message
    async fn handle_load_prefix(
        backend: &mut Backend,
        prefix: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Vec<Bytes>>,
    ) -> Result<(), StorageServiceError> {
        let result: Vec<Bytes> = backend
            .load_prefix(&prefix)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                message: format!("LoadPrefix {prefix:?}"),
            })
    }

    /// Handle remove message
    async fn handle_remove(
        backend: &mut Backend,
        key: Bytes,
        reply_channel: tokio::sync::oneshot::Sender<Option<Bytes>>,
    ) -> Result<(), StorageServiceError> {
        let result: Option<Bytes> = backend
            .remove(&key)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                message: format!("Remove {key:?}"),
            })
    }

    /// Handle store message
    async fn handle_store(
        backend: &mut Backend,
        key: Bytes,
        value: Bytes,
    ) -> Result<(), StorageServiceError> {
        backend
            .store(key, value)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))
    }

    /// Handle execute message
    async fn handle_execute(
        backend: &mut Backend,
        transaction: Backend::Transaction,
        reply_channel: tokio::sync::oneshot::Sender<
            <Backend::Transaction as StorageTransaction>::Result,
        >,
    ) -> Result<(), StorageServiceError> {
        let result = backend
            .execute(transaction)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))?;
        reply_channel
            .send(result)
            .map_err(|_| StorageServiceError::ReplyError {
                message: "Execute transaction".to_owned(),
            })
    }

    async fn handle_api_call(
        api_call: StorageApiRequest<Backend>,
        api_backend: &mut Backend,
    ) -> Result<(), StorageServiceError> {
        <StorageApiRequest<Backend> as StorageOperation<Backend>>::execute(api_call, api_backend)
            .await
            .map_err(|e| StorageServiceError::BackendError(e.into()))
    }
}

#[async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for StorageService<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
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

impl<Backend, RuntimeServiceId> ServiceData for StorageService<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
{
    type Settings = Backend::Settings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = StorageMsg<Backend>;
}
