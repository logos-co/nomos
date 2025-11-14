use core::pin::Pin;

use tokio::sync::oneshot;

use crate::{backend::KMSBackend, keys::secured_key::SecuredKey};

// TODO: Use [`AsyncFnMut`](https://doc.rust-lang.org/stable/std/ops/trait.AsyncFnMut.html#tymethod.async_call_mut) once it is stabilized.
type KMSOperator<Payload, Signature, PublicKey, KeyError, OperatorError> = Box<
    dyn FnOnce(
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
type KMSOperatorKey<Key, OperatorError> = KMSOperator<
    <Key as SecuredKey>::Payload,
    <Key as SecuredKey>::Signature,
    <Key as SecuredKey>::PublicKey,
    <Key as SecuredKey>::Error,
    OperatorError,
>;
pub type KMSOperatorBackend<Backend> =
    KMSOperatorKey<<Backend as KMSBackend>::Key, <Backend as KMSBackend>::Error>;

pub type KeyDescriptor<Backend> = (
    <Backend as KMSBackend>::KeyId,
    <<Backend as KMSBackend>::Key as SecuredKey>::PublicKey,
);

// TODO: Remove since we have an `execute` API that allows for signing.
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
        operator: KMSOperatorBackend<Backend>,
    },
}
