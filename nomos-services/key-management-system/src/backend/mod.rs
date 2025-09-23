#[cfg(feature = "preload")]
pub mod preload;

use crate::{KMSOperatorBackend, keys::SecuredKey};

#[async_trait::async_trait]
pub trait KMSBackend {
    type KeyId;
    type DataEncoding;
    type SupportedKey: SecuredKey<Self::DataEncoding>;
    type Settings;
    type Error;

    fn new(settings: Self::Settings) -> Self;

    fn register(
        &mut self,
        key_id: Self::KeyId,
        key_scheme: Self::SupportedKey,
    ) -> Result<Self::KeyId, Self::Error>;

    fn public_key(
        &self,
        key_id: Self::KeyId,
    ) -> Result<<Self::SupportedKey as SecuredKey<Self::DataEncoding>>::PublicKey, Self::Error>;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: Self::DataEncoding,
    ) -> Result<<Self::SupportedKey as SecuredKey<Self::DataEncoding>>::Signature, Self::Error>;

    async fn execute(
        &mut self,
        key_id: Self::KeyId,
        operator: KMSOperatorBackend<Self>,
    ) -> Result<(), Self::Error>;
}
