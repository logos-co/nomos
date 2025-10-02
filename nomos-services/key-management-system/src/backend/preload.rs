//! This module contains a simple test implementation of [`KMSBackend`] that
//! uses an alternate set of keys and encodings, different from the one provided
//! in this crate.
//!
//! It serves as a reference [`KMSBackend`] and demonstrates how encodings, keys
//! and adapters interact.

#[cfg(test)]
mod errors {
    use crate::{backend::preload::keys, keys::KeyError};

    #[derive(Debug)]
    pub enum PreloadKeyError {
        #[expect(dead_code, reason = "No errors are expected in these tests.")]
        KeyError(KeyError),
    }

    impl From<KeyError> for PreloadKeyError {
        fn from(value: KeyError) -> Self {
            Self::KeyError(value)
        }
    }

    #[derive(thiserror::Error, Debug)]
    pub enum PreloadBackendError {
        #[error("Key({0}) was not registered")]
        KeyNotRegistered(String),
        #[error("KeyType mismatch: {0:?} != {1:?}")]
        KeyTypeMismatch(keys::PreloadKeyKind, keys::PreloadKeyKind),
    }
}

#[cfg(test)]
mod encodings {
    use std::fmt::Debug;

    use bytes::Bytes;
    use ed25519_dalek::{Signature, VerifyingKey};

    pub enum PreloadEncoding {
        Bytes(Bytes),
    }

    impl PartialEq for PreloadEncoding {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Bytes(a), Self::Bytes(b)) => a == b,
            }
        }
    }

    impl Debug for PreloadEncoding {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Bytes(bytes) => {
                    write!(f, "PreloadEncodingFormat::Bytes({bytes:?})")
                }
            }
        }
    }

    impl From<Bytes> for PreloadEncoding {
        fn from(value: Bytes) -> Self {
            Self::Bytes(value)
        }
    }

    impl From<Signature> for PreloadEncoding {
        fn from(value: Signature) -> Self {
            let value_bytes = value.to_bytes();
            let bytes = Bytes::copy_from_slice(&value_bytes);
            Self::Bytes(bytes)
        }
    }

    impl From<VerifyingKey> for PreloadEncoding {
        fn from(value: VerifyingKey) -> Self {
            let bytes = Bytes::copy_from_slice(value.as_bytes());
            Self::Bytes(bytes)
        }
    }
}

#[cfg(test)]
mod keys {
    use std::fmt::{Debug, Formatter};

    use serde::{Deserialize, Serialize};
    use zeroize::ZeroizeOnDrop;

    use crate::{
        backend::preload::{encodings::PreloadEncoding, errors::PreloadKeyError},
        keys::{Ed25519Key, SecuredKey},
    };

    impl Debug for Ed25519Key {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Ed25519Key").finish()
        }
    }

    #[derive(Serialize, Deserialize, ZeroizeOnDrop, Debug)]
    pub enum PreloadKey {
        Ed25519(Ed25519Key),
    }

    impl PreloadKey {
        pub const fn key_type(&self) -> PreloadKeyKind {
            match self {
                Self::Ed25519(_) => PreloadKeyKind::Ed25519,
            }
        }
    }

    impl SecuredKey for PreloadKey {
        type Payload = PreloadEncoding;
        type Signature = PreloadEncoding;
        type PublicKey = PreloadEncoding;
        type Error = PreloadKeyError;

        fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
            match (self, payload) {
                (Self::Ed25519(key), PreloadEncoding::Bytes(bytes)) => key
                    .sign(bytes)
                    .map(PreloadEncoding::from)
                    .map_err(PreloadKeyError::KeyError),
            }
        }

        fn as_public_key(&self) -> Self::PublicKey {
            match self {
                Self::Ed25519(key) => key.as_public_key().into(),
            }
        }
    }

    /// This is used as a workaround (for this test) to call register without
    /// actual keys, only to verify whether a key of the same type was
    /// preloaded.
    ///
    /// To do this, [`PreloadKeyKind`] mimics the variants, `Encoding` and
    /// `Error` of [`PreloadKey`], so type remains accurate. Internally, the
    /// backend uses [`PreloadKey`] for the actual operations.
    ///
    /// TODO: Ideally, find a way to remove this.
    #[derive(Debug, PartialEq, Eq, ZeroizeOnDrop)]
    pub enum PreloadKeyKind {
        Ed25519,
    }

    impl SecuredKey for PreloadKeyKind {
        type Payload = <PreloadKey as SecuredKey>::Payload;
        type Signature = <PreloadKey as SecuredKey>::Signature;
        type PublicKey = <PreloadKey as SecuredKey>::PublicKey;
        type Error = <PreloadKey as SecuredKey>::Error;

        fn sign(&self, _data: &Self::Payload) -> Result<Self::Signature, Self::Error> {
            unimplemented!("Not needed.")
        }

        fn as_public_key(&self) -> Self::PublicKey {
            unimplemented!("Not needed.")
        }
    }
}

#[cfg(test)]
mod backends {
    use std::collections::HashMap;

    use overwatch::DynError;
    use serde::{Deserialize, Serialize};

    use crate::{
        KMSOperatorBackend, SecuredKey,
        backend::{
            KMSBackend,
            preload::{errors, keys},
        },
    };

    pub struct PreloadKMSBackend {
        keys: HashMap<String, keys::PreloadKey>,
    }

    /// This setting contains all [`PreloadKey`]s to be loaded into the
    /// [`PreloadKMSBackend`]. This implements [`serde::Serialize`] for users to
    /// populate the settings from bytes. The [`PreloadKey`] also implements
    /// [`zeroize::ZeroizeOnDrop`] for security.
    #[derive(Serialize, Deserialize)]
    pub struct PreloadKMSBackendSettings {
        pub keys: HashMap<String, keys::PreloadKey>,
    }

    #[async_trait::async_trait]
    impl KMSBackend for PreloadKMSBackend {
        type KeyId = String;
        type Key = keys::PreloadKeyKind;
        type Settings = PreloadKMSBackendSettings;
        type Error = DynError;

        fn new(settings: Self::Settings) -> Self {
            Self {
                keys: settings.keys,
            }
        }

        /// This function just checks if the `key_id` was preloaded
        /// successfully. It returns the `key_id` if the key was
        /// preloaded and the key type matches.
        fn register(
            &mut self,
            key_id: Self::KeyId,
            key_type: Self::Key,
        ) -> Result<Self::KeyId, Self::Error> {
            let key = self
                .keys
                .get(&key_id)
                .ok_or_else(|| errors::PreloadBackendError::KeyNotRegistered(key_id.clone()))?;
            if key.key_type() != key_type {
                return Err(
                    errors::PreloadBackendError::KeyTypeMismatch(key.key_type(), key_type).into(),
                );
            }
            Ok(key_id)
        }

        fn public_key(
            &self,
            key_id: Self::KeyId,
        ) -> Result<<Self::Key as SecuredKey>::PublicKey, Self::Error> {
            Ok(self
                .keys
                .get(&key_id)
                .ok_or(errors::PreloadBackendError::KeyNotRegistered(key_id))?
                .as_public_key())
        }

        fn sign(
            &self,
            key_id: Self::KeyId,
            payload: <Self::Key as SecuredKey>::Payload,
        ) -> Result<<Self::Key as SecuredKey>::Signature, Self::Error> {
            self.keys
                .get(&key_id)
                .ok_or(errors::PreloadBackendError::KeyNotRegistered(key_id))?
                .sign(&payload)
                .map_err(|error| DynError::from(format!("{error:?}")))
        }

        async fn execute(
            &mut self,
            key_id: Self::KeyId,
            mut operator: KMSOperatorBackend<Self>,
        ) -> Result<(), DynError> {
            let key = self
                .keys
                .get_mut(&key_id)
                .ok_or(errors::PreloadBackendError::KeyNotRegistered(key_id))?;

            operator(key)
                .await
                .map_err(|error| DynError::from(format!("{error:?}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bytes::{Bytes as RawBytes, Bytes};
    use rand::rngs::OsRng;

    use super::*;
    use crate::{
        KMSOperatorBackend, SecuredKey,
        backend::{
            KMSBackend,
            preload::{backends::PreloadKMSBackend, encodings::PreloadEncoding},
        },
        keys::Ed25519Key,
    };

    type BackendKey<'a, Backend> = dyn SecuredKey<
            Payload = <<Backend as KMSBackend>::Key as SecuredKey>::Payload,
            Signature = <<Backend as KMSBackend>::Key as SecuredKey>::Signature,
            PublicKey = <<Backend as KMSBackend>::Key as SecuredKey>::PublicKey,
            Error = <<Backend as KMSBackend>::Key as SecuredKey>::Error,
        > + 'a;
    fn noop_operator<Backend: KMSBackend>() -> KMSOperatorBackend<Backend> {
        Box::new(move |_: &BackendKey<Backend>| Box::pin(async move { Ok(()) }))
    }

    #[tokio::test]
    async fn preload_backend() {
        // Initialize a backend with a pre-generated key in the setting
        let key_id = "blend/1".to_owned();
        let key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let mut backend = PreloadKMSBackend::new(backends::PreloadKMSBackendSettings {
            keys: HashMap::from_iter(vec![(
                key_id.clone(),
                keys::PreloadKey::Ed25519(Ed25519Key(key.clone())),
            )]),
        });

        // Check if the key was preloaded successfully with the same key type.
        assert_eq!(
            backend
                .register(key_id.clone(), keys::PreloadKeyKind::Ed25519)
                .unwrap(),
            key_id
        );

        // Check if the backend key operations results are the same as the direct
        // operation on the key itself.
        let key = Ed25519Key(key.clone());
        let public_key = key.as_public_key().into();
        let backend_public_key = backend.public_key(key_id.clone()).unwrap();
        assert_eq!(backend_public_key, public_key);

        let payload = Bytes::from("data");
        let signature = key.sign(&payload).unwrap().into();
        let encoded_payload = PreloadEncoding::from(payload);
        let backend_signature = backend.sign(key_id.clone(), encoded_payload).unwrap();
        assert_eq!(backend_signature, signature);

        // Check if the execute function works as expected
        backend
            .execute(key_id.clone(), noop_operator::<PreloadKMSBackend>())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn key_not_registered() {
        let mut backend = PreloadKMSBackend::new(backends::PreloadKMSBackendSettings {
            keys: HashMap::new(),
        });

        let key_id = "blend/not_registered".to_owned();
        assert!(
            backend
                .register(key_id.clone(), keys::PreloadKeyKind::Ed25519)
                .is_err()
        );
        assert!(backend.public_key(key_id.clone()).is_err());
        let data = RawBytes::from("data");
        let encoded_data = PreloadEncoding::from(data);
        assert!(backend.sign(key_id.clone(), encoded_data).is_err());
        assert!(
            backend
                .execute(key_id, noop_operator::<PreloadKMSBackend>(),)
                .await
                .is_err()
        );
    }
}
