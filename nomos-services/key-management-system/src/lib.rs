use std::{
    fmt::{Debug, Display, Formatter},
    future::Future,
    pin::Pin,
};

use bytes::Bytes;
use log::error;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::oneshot;

use crate::{backend::KMSBackend, secure_key::SecuredKey};

mod backend;
mod keys;
mod secure_key;

// TODO: Use [`AsyncFnMut`](https://doc.rust-lang.org/stable/std/ops/trait.AsyncFnMut.html#tymethod.async_call_mut) once it is stabilized.
pub type KMSOperator = Box<
    dyn FnMut(
            &mut dyn SecuredKey,
        ) -> Pin<Box<dyn Future<Output = Result<(), DynError>> + Send + Sync>>
        + Send
        + Sync,
>;

pub enum KMSMessage<Backend>
where
    Backend: KMSBackend,
{
    Register {
        key_id: Backend::KeyId,
        key_type: Backend::SupportedKeyTypes,
        reply_channel: oneshot::Sender<Backend::KeyId>,
    },
    PublicKey {
        key_id: Backend::KeyId,
        reply_channel: oneshot::Sender<Bytes>,
    },
    Sign {
        key_id: Backend::KeyId,
        data: Bytes,
        reply_channel: oneshot::Sender<Bytes>,
    },
    Execute {
        key_id: Backend::KeyId,
        operator: KMSOperator,
    },
}

impl<Backend> Debug for KMSMessage<Backend>
where
    Backend: KMSBackend,
    Backend::KeyId: Debug,
    Backend::SupportedKeyTypes: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Register {
                key_id, key_type, ..
            } => {
                write!(
                    f,
                    "KMS-Register {{ KeyId: {key_id:?}, KeyScheme: {key_type:?} }}"
                )
            }
            Self::PublicKey { key_id, .. } => {
                write!(f, "KMS-PublicKey {{ KeyId: {key_id:?} }}")
            }
            Self::Sign { key_id, .. } => {
                write!(f, "KMS-Sign {{ KeyId: {key_id:?} }}")
            }
            Self::Execute { .. } => {
                write!(f, "KMS-Execute")
            }
        }
    }
}

#[derive(Clone)]
pub struct KMSServiceSettings<BackendSettings> {
    backend_settings: BackendSettings,
}

pub struct KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + 'static,
    Backend::KeyId: Debug,
    Backend::SupportedKeyTypes: Debug,
    Backend::Settings: Clone,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

impl<Backend, RuntimeServiceId> ServiceData for KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + 'static,
    Backend::KeyId: Debug,
    Backend::SupportedKeyTypes: Debug,
    Backend::Settings: Clone,
{
    type Settings = KMSServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = KMSMessage<Backend>;
}

#[async_trait::async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + Send + 'static,
    Backend::KeyId: Debug + Send,
    Backend::SupportedKeyTypes: Debug + Send,
    Backend::Settings: Clone + Send + Sync,
    RuntimeServiceId: AsServiceId<Self> + Display + Send,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let KMSServiceSettings { backend_settings } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let backend = Backend::new(backend_settings);
        Ok(Self {
            backend,
            service_resources_handle,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    status_updater,
                    ..
                },
            mut backend,
        } = self;

        status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(msg) = inbound_relay.recv().await {
            Self::handle_kms_message(msg, &mut backend).await;
        }

        Ok(())
    }
}

impl<Backend, RuntimeServiceId> KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + 'static,
    Backend::KeyId: Debug,
    Backend::SupportedKeyTypes: Debug,
    Backend::Settings: Clone,
{
    async fn handle_kms_message(msg: KMSMessage<Backend>, backend: &mut Backend) {
        match msg {
            KMSMessage::Register {
                key_id,
                key_type,
                reply_channel,
            } => {
                let Ok(key_id) = backend.register(key_id, key_type) else {
                    panic!("A key could not be registered");
                };
                if let Err(_key_id) = reply_channel.send(key_id) {
                    error!("Could not reply key_id for register request");
                }
            }
            KMSMessage::PublicKey {
                key_id,
                reply_channel,
            } => {
                let Ok(pk_bytes) = backend.public_key(key_id) else {
                    panic!("Requested public key for nonexistent KeyId");
                };
                if let Err(_pk_bytes) = reply_channel.send(pk_bytes) {
                    error!("Could not reply public key to request channel");
                }
            }
            KMSMessage::Sign {
                key_id,
                data,
                reply_channel,
            } => {
                let Ok(signature) = backend.sign(key_id, data) else {
                    panic!("Could not sign ")
                };
                if let Err(_signature) = reply_channel.send(signature) {
                    error!("Could not reply public key to request channel");
                }
            }
            KMSMessage::Execute { key_id, operator } => {
                backend
                    .execute(key_id, operator)
                    .await
                    .expect("Could not execute operator");
            }
        }
    }
}
