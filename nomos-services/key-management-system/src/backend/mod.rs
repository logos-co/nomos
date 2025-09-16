#[cfg(feature = "preload")]
pub mod preload;

use crate::{KMSOperatorKey, keys::SecuredKey};

#[async_trait::async_trait]
pub trait KMSBackend {
    type KeyId;
    type SupportedKey: SecuredKey;
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
    ) -> Result<<Self::SupportedKey as SecuredKey>::EncodingFormat, Self::Error>;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::EncodingFormat,
    ) -> Result<<Self::SupportedKey as SecuredKey>::EncodingFormat, Self::Error>;

    async fn execute(
        &mut self,
        key_id: Self::KeyId,
        operator: KMSOperatorKey<Self::SupportedKey>,
    ) -> Result<(), Self::Error>;
}
