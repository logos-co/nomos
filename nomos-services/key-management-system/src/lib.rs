use crate::backend::KMSBackend;
use crate::secure_key::SecuredKey;
use bytes::Bytes;
use either::Either;
use overwatch_rs::services::handle::ServiceStateHandle;
use overwatch_rs::services::relay::RelayMessage;
use overwatch_rs::services::state::{NoOperator, NoState};
use overwatch_rs::services::{ServiceCore, ServiceData, ServiceId};
use overwatch_rs::DynError;
use std::fmt::{Debug, Formatter};
use std::future::Future;
use tokio::sync::oneshot;

mod backend;
mod secure_key;

const KMS_TAG: ServiceId = "KMS";

// TODO: Use [`AsyncFnMut`](https://doc.rust-lang.org/stable/std/ops/trait.AsyncFnMut.html#tymethod.async_call_mut) once it is stabilized.
pub type KMSOperator =
    Box<dyn FnMut(&mut dyn SecuredKey) -> Box<dyn Future<Output = Result<(), DynError>>>>;

pub enum KMSMessage<Backend>
where
    Backend: KMSBackend,
{
    Register {
        key: Either<Backend::SupportedKeys, Backend::KeyId>,
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
        operator: KMSOperator,
    },
}

impl<Backend> Debug for KMSMessage<Backend>
where
    Backend: KMSBackend,
    Backend::KeyId: Debug,
    Backend::SupportedKeys: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            KMSMessage::Register {
                key: Either::Right(key_id),
                ..
            } => {
                write!(f, "KMS-Register {{ KeyId: {key_id:?} }}")
            }
            KMSMessage::Register {
                key: Either::Left(key_type),
                ..
            } => {
                write!(f, "KMS-Register {{ KeyType: {key_type:?} }}")
            }
            KMSMessage::PublicKey { key_id, .. } => {
                write!(f, "KMS-PublicKey {{ KeyId: {key_id:?} }}")
            }
            KMSMessage::Sign { key_id, .. } => {
                write!(f, "KMS-Sign {{ KeyId: {key_id:?} }}")
            }
            KMSMessage::Execute { .. } => {
                write!(f, "KMS-Execute")
            }
        }
    }
}

impl<B: backend::KMSBackend + 'static> RelayMessage for KMSMessage<B> {}

#[derive(Clone)]
pub struct KMSServiceSettings<BackendSettings> {
    backend_settings: BackendSettings,
}

pub struct KMSService<Backend> {
    backend: Backend,
}

impl<Backend> ServiceData for KMSService<Backend>
where
    Backend: KMSBackend + 'static,
    Backend::KeyId: Debug,
    Backend::SupportedKeys: Debug,
    Backend::Settings: Clone,
{
    const SERVICE_ID: ServiceId = KMS_TAG;
    type Settings = KMSServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = KMSMessage<Backend>;
}

#[async_trait::async_trait]
impl<Backend> ServiceCore for KMSService<Backend>
where
    Backend: KMSBackend + Send + 'static,
    Backend::KeyId: Debug,
    Backend::SupportedKeys: Debug,
    Backend::Settings: Clone,
{
    fn init(service_state: ServiceStateHandle<Self>) -> Result<Self, DynError> {
        let KMSServiceSettings { backend_settings } =
            service_state.settings_reader.get_updated_settings();
        let backend = Backend::new(backend_settings);
        Ok(Self { backend })
    }

    async fn run(self) -> Result<(), DynError> {
        todo!()
    }
}
