pub mod api;
pub mod backend;
pub mod keys;

use std::{
    fmt::{Debug, Display},
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

#[derive(Debug)]
pub enum KMSSigningStrategy<KeyId> {
    Single(KeyId),
    Multi(Vec<KeyId>),
}

pub enum KMSMessage<Backend>
where
    Backend: KMSBackend,
{
    Register {
        key_id: Backend::KeyId,
        key_type: Backend::Key,
        reply_channel: oneshot::Sender<Result<KeyDescriptor<Backend>, Backend::Error>>,
    },
    PublicKey {
        key_id: Backend::KeyId,
        reply_channel:
            oneshot::Sender<Result<<Backend::Key as SecuredKey>::PublicKey, Backend::Error>>,
    },
    Sign {
        signing_strategy: KMSSigningStrategy<Backend::KeyId>,
        payload: <Backend::Key as SecuredKey>::Payload,
        reply_channel:
            oneshot::Sender<Result<<Backend::Key as SecuredKey>::Signature, Backend::Error>>,
    },
    Execute {
        key_id: Backend::KeyId,
        operator: KMSOperatorKey<Backend::Key, <Backend as KMSBackend>::Error>,
    },
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
    type Settings = Backend::Settings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = KMSMessage<Backend>;
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
        let backend_settings = service_resources_handle
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
    async fn handle_kms_message(message: KMSMessage<Backend>, backend: &mut Backend) {
        match message {
            KMSMessage::Register {
                key_id,
                key_type,
                reply_channel,
            } => {
                if let Err(e) = backend.register(&key_id, key_type) {
                    if reply_channel.send(Err(e)).is_err() {
                        error!("Could not send backend key registration error to caller.");
                    }
                    return;
                }

                let pk_bytes_result = backend.public_key(&key_id).map(|pk| (key_id.clone(), pk));
                if reply_channel.send(pk_bytes_result).is_err() {
                    error!("Could not reply to the public key request channel");
                }
            }
            KMSMessage::PublicKey {
                key_id,
                reply_channel,
            } => {
                let pk_bytes_result = backend.public_key(&key_id);
                if reply_channel.send(pk_bytes_result).is_err() {
                    error!("Could not reply to the public key request channel");
                }
            }
            KMSMessage::Sign {
                signing_strategy,
                payload,
                reply_channel,
            } => {
                let signature_result = match signing_strategy {
                    KMSSigningStrategy::Single(key) => backend.sign(&key, payload),
                    KMSSigningStrategy::Multi(keys) => {
                        backend.sign_multiple(keys.as_slice(), payload)
                    }
                };
                if reply_channel.send(signature_result).is_err() {
                    error!("Could not reply to the public key request channel");
                }
            }
            KMSMessage::Execute { key_id, operator } => {
                let _ = backend.execute(&key_id, operator).await.inspect_err(|e| {
                    error!("Failed to execute operator with key ID {key_id:?}. Error: {e:?}");
                });
            }
        }
    }
}
