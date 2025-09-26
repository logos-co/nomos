mod backend;
mod keys;

use std::{
    fmt::{Debug, Display, Formatter},
    pin::Pin,
};

use log::error;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use tokio::sync::oneshot;

use crate::{backend::KMSBackend, keys::secured_key::SecuredKey};

// TODO: Use [`AsyncFnMut`](https://doc.rust-lang.org/stable/std/ops/trait.AsyncFnMut.html#tymethod.async_call_mut) once it is stabilized.
pub type KMSOperator<Payload, Signature, PublicKey, KeyError, OperatorError> = Box<
    dyn FnMut(
            &dyn SecuredKey<
                Payload = Payload,
                Signature = Signature,
                PublicKey = PublicKey,
                Error = KeyError,
            >,
        ) -> Pin<Box<dyn Future<Output = Result<(), OperatorError>> + Send + Sync>>
        + Send
        + Sync,
>;

pub type KMSOperatorKey<Key, OperatorError> = KMSOperator<
    <Key as SecuredKey>::Payload,
    <Key as SecuredKey>::Signature,
    <Key as SecuredKey>::PublicKey,
    <Key as SecuredKey>::Error,
    OperatorError,
>;

pub type KMSOperatorBackend<Backend> =
    KMSOperatorKey<<Backend as KMSBackend>::Key, <Backend as KMSBackend>::Error>;

type KeyDescriptor<Backend> = (
    <Backend as KMSBackend>::KeyId,
    <<Backend as KMSBackend>::Key as SecuredKey>::PublicKey,
);

pub enum KMSMessage<Backend, Payload, Signature, PublicKey, KeyError, OperatorError>
where
    Backend: KMSBackend,
{
    Register {
        key_id: Backend::KeyId,
        key_type: Backend::Key,
        reply_channel: oneshot::Sender<KeyDescriptor<Backend>>,
    },
    PublicKey {
        key_id: Backend::KeyId,
        reply_channel: oneshot::Sender<PublicKey>,
    },
    Sign {
        key_id: Backend::KeyId,
        data: Payload,
        reply_channel: oneshot::Sender<Signature>,
    },
    Execute {
        key_id: Backend::KeyId,
        operator: KMSOperator<Payload, Signature, PublicKey, KeyError, OperatorError>,
    },
}

pub type KMSMessageBackend<Backend> = KMSMessage<
    Backend,
    <<Backend as KMSBackend>::Key as SecuredKey>::Payload,
    <<Backend as KMSBackend>::Key as SecuredKey>::Signature,
    <<Backend as KMSBackend>::Key as SecuredKey>::PublicKey,
    <<Backend as KMSBackend>::Key as SecuredKey>::Error,
    <Backend as KMSBackend>::Error,
>;

impl<Backend> Debug for KMSMessageBackend<Backend>
where
    Backend: KMSBackend,
    Backend::KeyId: Debug,
    Backend::Key: Debug,
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
    Backend::Key: Debug,
    Backend::Settings: Clone,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

impl<Backend, RuntimeServiceId> ServiceData for KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + 'static,
    Backend::KeyId: Debug,
    Backend::Key: Debug,
    Backend::Settings: Clone,
{
    type Settings = KMSServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = KMSMessageBackend<Backend>;
}

#[async_trait::async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for KMSService<Backend, RuntimeServiceId>
where
    Backend: KMSBackend + Send + 'static,
    Backend::KeyId: Clone + Debug + Send,
    Backend::Key: Debug + Send,
    <Backend::Key as SecuredKey>::Payload: Send,
    <Backend::Key as SecuredKey>::Signature: Send,
    <Backend::Key as SecuredKey>::PublicKey: Send,
    Backend::Settings: Clone + Send + Sync,
    Backend::Error: Debug + Send,
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
    Backend::KeyId: Debug + Clone,
    Backend::Key: Debug,
    Backend::Settings: Clone,
    Backend::Error: Debug,
{
    async fn handle_kms_message(message: KMSMessageBackend<Backend>, backend: &mut Backend) {
        match message {
            KMSMessage::Register {
                key_id,
                key_type,
                reply_channel,
            } => {
                let Ok(key_id) = backend.register(key_id, key_type) else {
                    panic!("A key could not be registered");
                };
                let Ok(key_public_key) = backend.public_key(key_id.clone()) else {
                    panic!("Requested public key for nonexistent KeyId");
                };
                if let Err(_key_descriptor) = reply_channel.send((key_id, key_public_key)) {
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
                    error!("Could not reply to the public key request channel");
                }
            }
            KMSMessage::Sign {
                key_id,
                data,
                reply_channel,
            } => {
                let Ok(signature) = backend.sign(key_id, data) else {
                    panic!("Could not sign.")
                };
                if let Err(_signature) = reply_channel.send(signature) {
                    error!("Could not reply to the public key request channel");
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
