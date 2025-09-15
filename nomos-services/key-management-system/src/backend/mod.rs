#[cfg(feature = "preload")]
pub mod preload;

use crate::keys::secured_key::SecuredKey;
use crate::KMSOperator;

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

    fn public_key(&self, key_id: Self::KeyId) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error>;

    fn sign(&self, key_id: Self::KeyId, data: <Self::SupportedKey as SecuredKey>::Encoding) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error>;

    async fn execute(&mut self, key_id: Self::KeyId, mut operator: KMSOperator<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error>) -> Result<(), Self::Error>;
}
