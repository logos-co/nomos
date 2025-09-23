//! This module contains a simple test implementation of [`KMSBackend`] that
//! uses an alternate set of keys and encodings, different from the one provided
//! in this crate.
//!
//! It serves as a reference [`KMSBackend`] and demonstrates how encodings, keys
//! and adapters interact.

#[cfg(test)]
mod errors {
    use super::*;
    use crate::keys::KeyError;

    #[derive(Debug)]
    pub enum PreloadKeyError {
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

    use crate::encodings::Bytes;

    pub enum PreloadEncodingFormat {
        Bytes(Bytes),
    }

    impl PartialEq for PreloadEncodingFormat {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Bytes(a), Self::Bytes(b)) => a.as_bytes() == b.as_bytes(),
            }
        }
    }

    impl Debug for PreloadEncodingFormat {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Bytes(bytes) => {
                    write!(f, "PreloadEncodingFormat::Bytes({:?})", bytes.as_bytes())
                }
            }
        }
    }

    impl From<Bytes> for PreloadEncodingFormat {
        fn from(value: Bytes) -> Self {
            Self::Bytes(value)
        }
    }

    impl From<bytes::Bytes> for PreloadEncodingFormat {
        fn from(value: bytes::Bytes) -> Self {
            Self::Bytes(Bytes::from(value))
        }
    }
}

#[cfg(test)]
mod keys {
    use std::fmt::{Debug, Formatter};

    use serde::{Deserialize, Serialize};
    use zeroize::ZeroizeOnDrop;

    use crate::{
        backend::preload::{encodings::PreloadEncodingFormat, errors::PreloadKeyError},
        encodings::Bytes,
        keys::{Ed25519Key, SecuredKey},
    };

    impl Debug for Ed25519Key {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Ed25519Key").finish()
        }
    }

    impl SecuredKey<PreloadEncodingFormat> for Ed25519Key {
        type Signature = PreloadEncodingFormat;
        type PublicKey = PreloadEncodingFormat;
        type Error = PreloadKeyError;

        fn sign(&self, data: &PreloadEncodingFormat) -> Result<Self::Signature, Self::Error> {
            match data {
                PreloadEncodingFormat::Bytes(bytes) => {
                    <Self as SecuredKey<Bytes>>::sign(self, bytes)
                        .map(PreloadEncodingFormat::from)
                        .map_err(PreloadKeyError::from)
                }
            }
        }

        fn as_public_key(&self) -> Self::PublicKey {
            <Self as SecuredKey<Bytes>>::as_public_key(self).into()
        }
    }

    #[derive(Serialize, Deserialize, ZeroizeOnDrop, Debug)]
    pub enum PreloadKey {
        Ed25519(Ed25519Key),
    }

    impl PreloadKey {
        pub fn key_type(&self) -> PreloadKeyKind {
            match self {
                Self::Ed25519(_) => PreloadKeyKind::Ed25519,
            }
        }
    }

    impl SecuredKey<PreloadEncodingFormat> for PreloadKey {
        type Signature = PreloadEncodingFormat;
        type PublicKey = PreloadEncodingFormat;
        type Error = PreloadKeyError;

        fn sign(&self, data: &PreloadEncodingFormat) -> Result<Self::Signature, Self::Error> {
            match self {
                Self::Ed25519(key) => {
                    <Ed25519Key as SecuredKey<PreloadEncodingFormat>>::sign(key, data)
                }
            }
        }

        fn as_public_key(&self) -> Self::PublicKey {
            match self {
                Self::Ed25519(key) => {
                    <Ed25519Key as SecuredKey<PreloadEncodingFormat>>::as_public_key(key)
                }
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

    impl SecuredKey<PreloadEncodingFormat> for PreloadKeyKind {
        type Signature = <PreloadKey as SecuredKey<PreloadEncodingFormat>>::Signature;
        type PublicKey = <PreloadKey as SecuredKey<PreloadEncodingFormat>>::PublicKey;
        type Error = <PreloadKey as SecuredKey<PreloadEncodingFormat>>::Error;

        fn sign(&self, _data: &PreloadEncodingFormat) -> Result<Self::Signature, Self::Error> {
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

    use super::*;
    use crate::{
        KMSOperatorBackend, SecuredKey,
        backend::{KMSBackend, preload::encodings::PreloadEncodingFormat},
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
        type DataEncoding = PreloadEncodingFormat;
        type SupportedKey = keys::PreloadKeyKind;
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
            key_type: Self::SupportedKey,
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
        ) -> Result<<Self::SupportedKey as SecuredKey<PreloadEncodingFormat>>::PublicKey, Self::Error>
        {
            Ok(self
                .keys
                .get(&key_id)
                .ok_or(errors::PreloadBackendError::KeyNotRegistered(key_id))?
                .as_public_key())
        }

        fn sign(
            &self,
            key_id: Self::KeyId,
            data: PreloadEncodingFormat,
        ) -> Result<<Self::SupportedKey as SecuredKey<PreloadEncodingFormat>>::Signature, Self::Error>
        {
            self.keys
                .get(&key_id)
                .ok_or(errors::PreloadBackendError::KeyNotRegistered(key_id))?
                .sign(&data)
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

    use bytes::Bytes as RawBytes;
    use rand::rngs::OsRng;

    use super::*;
    use crate::{
        SecuredKey,
        backend::{
            KMSBackend as _,
            preload::{encodings::PreloadEncodingFormat, errors::PreloadKeyError},
        },
        encodings::Bytes,
        keys::Ed25519Key,
    };

    #[tokio::test]
    async fn preload_backend() {
        // Initialize a backend with a pre-generated key in the setting
        let key_id = "blend/1".to_owned();
        let key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let mut backend = backends::PreloadKMSBackend::new(backends::PreloadKMSBackendSettings {
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
        let pk = <Ed25519Key as SecuredKey<Bytes>>::as_public_key(&key);
        let encoded_pk = PreloadEncodingFormat::from(pk);
        let backend_pk = backend.public_key(key_id.clone()).unwrap();
        assert_eq!(backend_pk, encoded_pk);

        let data = RawBytes::from("data");
        let wrapped_data = Bytes::from(data);
        let signature = key.sign(&wrapped_data).unwrap();
        let encoded_signature = PreloadEncodingFormat::from(signature);

        let encoded_data = PreloadEncodingFormat::from(wrapped_data);
        let backend_data = backend.sign(key_id.clone(), encoded_data).unwrap();
        assert_eq!(backend_data, encoded_signature);

        // Check if the execute function works as expected
        backend
            .execute(
                key_id.clone(),
                Box::new(
                    move |_: &mut dyn SecuredKey<
                        PreloadEncodingFormat,
                        PublicKey = PreloadEncodingFormat,
                        Signature = PreloadEncodingFormat,
                        Error = PreloadKeyError,
                    >| Box::pin(async move { Ok(()) }),
                ),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn key_not_registered() {
        let mut backend = backends::PreloadKMSBackend::new(backends::PreloadKMSBackendSettings {
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
        let encoded_data = PreloadEncodingFormat::from(data);
        assert!(backend.sign(key_id.clone(), encoded_data).is_err());
        assert!(
            backend
                .execute(
                    key_id,
                    Box::new(
                        move |_: &mut dyn SecuredKey<
                            PreloadEncodingFormat,
                            PublicKey = PreloadEncodingFormat,
                            Signature = PreloadEncodingFormat,
                            Error = PreloadKeyError,
                        >| Box::pin(async move { Ok(()) })
                    ),
                )
                .await
                .is_err()
        );
    }
}
