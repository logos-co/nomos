//! This module contains a simple implementation of [`KMSBackend`] where keys
//! are preloaded from config file.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{KMSOperatorBackend, SecuredKey, backend::KMSBackend, keys};

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum PreloadBackendError {
    #[error(transparent)]
    KeyError(#[from] keys::errors::KeyError),
    #[error("KeyId ({0:?}) is not registered")]
    NotRegisteredKeyId(String),
    #[error("KeyId {0} is already registered")]
    AlreadRegisteredKeyId(String),
}

pub struct PreloadKMSBackend {
    keys: HashMap<String, keys::Key>,
}

/// This setting contains all [`Key`]s to be loaded into the
/// [`PreloadKMSBackend`]. This implements [`serde::Serialize`] for users to
/// populate the settings from bytes.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PreloadKMSBackendSettings {
    pub keys: HashMap<String, keys::Key>,
}

#[async_trait::async_trait]
impl KMSBackend for PreloadKMSBackend {
    type KeyId = String;
    type Key = keys::Key;
    type Settings = PreloadKMSBackendSettings;
    type Error = PreloadBackendError;

    fn new(settings: Self::Settings) -> Self {
        Self {
            keys: settings.keys,
        }
    }

    // Keys created after initialization will be held in memory but not persisted
    // across restarts
    fn register(&mut self, key_id: &Self::KeyId, key: Self::Key) -> Result<(), Self::Error> {
        if self.keys.contains_key(key_id) {
            return Err(PreloadBackendError::AlreadRegisteredKeyId(key_id.clone()));
        }
        self.keys.insert(key_id.clone(), key);

        Ok(())
    }

    fn public_key(
        &self,
        key_id: &Self::KeyId,
    ) -> Result<<Self::Key as SecuredKey>::PublicKey, Self::Error> {
        Ok(self
            .keys
            .get(key_id)
            .ok_or_else(|| PreloadBackendError::NotRegisteredKeyId(key_id.to_owned()))?
            .as_public_key())
    }

    fn sign(
        &self,
        key_id: &Self::KeyId,
        payload: <Self::Key as SecuredKey>::Payload,
    ) -> Result<<Self::Key as SecuredKey>::Signature, Self::Error> {
        Ok(self
            .keys
            .get(key_id)
            .ok_or_else(|| PreloadBackendError::NotRegisteredKeyId(key_id.to_owned()))?
            .sign(&payload)?)
    }

    fn sign_multiple(
        &self,
        key_ids: &[Self::KeyId],
        payload: <Self::Key as SecuredKey>::Payload,
    ) -> Result<<Self::Key as SecuredKey>::Signature, Self::Error> {
        let keys = key_ids
            .iter()
            .map(|key_id| {
                self.keys
                    .get(key_id)
                    .ok_or_else(|| PreloadBackendError::NotRegisteredKeyId(key_id.to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self::Key::sign_multiple(&keys, &payload)?)
    }

    async fn execute(
        &mut self,
        key_id: &Self::KeyId,
        mut operator: KMSOperatorBackend<Self>,
    ) -> Result<(), Self::Error> {
        let key = self
            .keys
            .get_mut(key_id)
            .ok_or_else(|| PreloadBackendError::NotRegisteredKeyId(key_id.to_owned()))?;

        operator(key).await
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Bytes as RawBytes, Bytes};
    use num_bigint::BigUint;
    use rand::rngs::OsRng;
    use zksign::SecretKey;

    use super::*;
    use crate::keys::{Ed25519Key, Key, PayloadEncoding, ZkKey};

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
        let key = Key::Ed25519(Ed25519Key(ed25519_dalek::SigningKey::generate(&mut OsRng)));
        let mut backend = PreloadKMSBackend::new(PreloadKMSBackendSettings {
            keys: HashMap::from_iter([(key_id.clone(), key.clone())]),
        });

        // Check if the key was preloaded successfully with the same key type.
        assert_eq!(
            backend.register(&key_id, key.clone()).unwrap_err(),
            PreloadBackendError::AlreadRegisteredKeyId(key_id.clone())
        );

        let public_key = key.as_public_key();
        let backend_public_key = backend.public_key(&key_id).unwrap();
        assert_eq!(backend_public_key, public_key);

        let payload = PayloadEncoding::Ed25519(Bytes::from("data"));
        let signature = key.sign(&payload).unwrap();
        let backend_signature = backend.sign(&key_id, payload).unwrap();
        assert_eq!(backend_signature, signature);

        // Check if the execute function works as expected
        backend
            .execute(&key_id, noop_operator::<PreloadKMSBackend>())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn key_not_registered() {
        let mut backend = PreloadKMSBackend::new(PreloadKMSBackendSettings {
            keys: HashMap::new(),
        });

        let key_id = "blend/not_registered".to_owned();
        let key = Key::Ed25519(Ed25519Key(ed25519_dalek::SigningKey::generate(&mut OsRng)));

        // Fetching public key fails
        assert_eq!(
            backend.public_key(&key_id).unwrap_err(),
            PreloadBackendError::NotRegisteredKeyId(key_id.clone())
        );

        // Signing with a key id fails
        let data = RawBytes::from("data");
        let encoded_data = PayloadEncoding::Ed25519(data);
        assert_eq!(
            backend.sign(&key_id, encoded_data).unwrap_err(),
            PreloadBackendError::NotRegisteredKeyId(key_id.clone())
        );

        // Excuting with a key id fails
        assert_eq!(
            backend
                .execute(&key_id, noop_operator::<PreloadKMSBackend>())
                .await
                .unwrap_err(),
            PreloadBackendError::NotRegisteredKeyId(key_id.clone()),
        );

        // Registering the key works
        assert_eq!(backend.register(&key_id, key), Ok(()));
    }

    #[test]
    fn serde_keys_from_yaml() {
        let preloaded_keys = PreloadKMSBackendSettings {
            keys: [
                (
                    "test1".into(),
                    Key::Ed25519(Ed25519Key(ed25519_dalek::SigningKey::generate(&mut OsRng))),
                ),
                (
                    "test2".into(),
                    Key::Zk(ZkKey(SecretKey::from(BigUint::from_bytes_le(&[1u8; 32])))),
                ),
            ]
            .into(),
        };

        let mut serialized_ouput = Vec::new();
        serde_yaml::to_writer(&mut serialized_ouput, &preloaded_keys).unwrap();

        let deserialized_keys: PreloadKMSBackendSettings =
            serde_yaml::from_slice(&serialized_ouput).unwrap();

        assert_eq!(preloaded_keys.keys.len(), deserialized_keys.keys.len());
        let original_key = preloaded_keys.keys.keys().next().unwrap();
        assert!(deserialized_keys.keys.contains_key(original_key));
    }
}
