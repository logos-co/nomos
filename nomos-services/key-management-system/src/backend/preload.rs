#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bytes::Bytes;
    use overwatch::DynError;
    use rand::rngs::OsRng;
    use serde::{Deserialize, Serialize};
    use zeroize::ZeroizeOnDrop;

    use crate::{
        KMSOperator, backend::KMSBackend, keys::ed25519::Ed25519Key, secure_key::SecuredKey,
    };

    pub struct PreloadKMSBackend {
        keys: HashMap<String, Key>,
    }

    /// This settings contain all [`Key`]s to be loaded into the
    /// [`PreloadKMSBackend`]. This implements [`serde::Serialize`] for users to
    /// populate the settings from bytes. The [`Key`] also implements
    /// [`zeroize::ZeroizeOnDrop`] for security.
    #[derive(Serialize, Deserialize)]
    pub struct PreloadKMSBackendSettings {
        keys: HashMap<String, Key>,
    }

    #[async_trait::async_trait]
    impl KMSBackend for PreloadKMSBackend {
        type SupportedKeyTypes = SupportedKeyTypes;
        type KeyId = String;
        type Settings = PreloadKMSBackendSettings;

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
            key_type: Self::SupportedKeyTypes,
        ) -> Result<Self::KeyId, DynError> {
            let key = self
                .keys
                .get(&key_id)
                .ok_or_else(|| Error::KeyNotRegistered(key_id.clone()))?;
            if key.key_type() != key_type {
                return Err(Error::KeyTypeMismatch(key.key_type(), key_type).into());
            }
            Ok(key_id)
        }

        fn public_key(&self, key_id: Self::KeyId) -> Result<Bytes, DynError> {
            Ok(self
                .keys
                .get(&key_id)
                .ok_or(Error::KeyNotRegistered(key_id))?
                .as_pk())
        }

        fn sign(&self, key_id: Self::KeyId, data: Bytes) -> Result<Bytes, DynError> {
            self.keys
                .get(&key_id)
                .ok_or(Error::KeyNotRegistered(key_id))?
                .sign(data)
        }

        async fn execute(
            &mut self,
            key_id: Self::KeyId,
            mut op: KMSOperator,
        ) -> Result<(), DynError> {
            op(self
                .keys
                .get_mut(&key_id)
                .ok_or(Error::KeyNotRegistered(key_id))?)
            .await
        }
    }

    // This enum won't be used outside of this module
    // because [`PreloadKMSBackend`] doesn't support generating new keys.
    #[derive(Debug, PartialEq, Eq)]
    pub enum SupportedKeyTypes {
        Ed25519,
    }

    #[derive(Serialize, Deserialize, ZeroizeOnDrop)]
    enum Key {
        Ed25519(Ed25519Key),
    }

    impl SecuredKey for Key {
        fn sign(&self, data: Bytes) -> Result<Bytes, DynError> {
            match self {
                Self::Ed25519(key) => key.sign(data),
            }
        }

        fn as_pk(&self) -> Bytes {
            match self {
                Self::Ed25519(key) => key.as_pk(),
            }
        }
    }

    impl Key {
        const fn key_type(&self) -> SupportedKeyTypes {
            match self {
                Self::Ed25519(_) => SupportedKeyTypes::Ed25519,
            }
        }
    }

    #[derive(thiserror::Error, Debug)]
    pub enum Error {
        #[error("Key({0}) was not registered")]
        KeyNotRegistered(String),
        #[error("KeyType mismatch: {0:?} != {1:?}")]
        KeyTypeMismatch(SupportedKeyTypes, SupportedKeyTypes),
    }

    #[tokio::test]
    async fn preload_backend() {
        // Initialize a backend with a pre-generated key in the setting
        let key_id = "blend/1".to_owned();
        let mut rng = OsRng;
        let key = ed25519_dalek::SigningKey::generate(&mut rng);
        let mut backend = PreloadKMSBackend::new(PreloadKMSBackendSettings {
            keys: HashMap::from_iter(vec![(
                key_id.clone(),
                Key::Ed25519(Ed25519Key(key.clone())),
            )]),
        });

        // Check if the key was preloaded successfully with the same key type.
        assert_eq!(
            backend
                .register(key_id.clone(), SupportedKeyTypes::Ed25519)
                .unwrap(),
            key_id
        );

        // Check if the result of key operations of the backend are the same as
        // the direct operation on the key itself.
        let key = Ed25519Key(key.clone());
        assert_eq!(backend.public_key(key_id.clone()).unwrap(), key.as_pk());
        let data = Bytes::from("data");
        assert_eq!(
            backend.sign(key_id.clone(), data.clone()).unwrap(),
            key.sign(data).unwrap()
        );

        // Check if the execute function works as expected
        backend
            .execute(
                key_id.clone(),
                Box::new(move |_: &mut dyn SecuredKey| Box::pin(async move { Ok(()) })),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn key_not_registered() {
        let mut backend = PreloadKMSBackend::new(PreloadKMSBackendSettings {
            keys: HashMap::new(),
        });

        let key_id = "blend/not_registered".to_owned();
        assert!(
            backend
                .register(key_id.clone(), SupportedKeyTypes::Ed25519)
                .is_err()
        );
        assert!(backend.public_key(key_id.clone()).is_err());
        assert!(backend.sign(key_id.clone(), Bytes::from("data")).is_err());
        assert!(
            backend
                .execute(
                    key_id,
                    Box::new(move |_: &mut dyn SecuredKey| Box::pin(async move { Ok(()) })),
                )
                .await
                .is_err()
        );
    }
}
