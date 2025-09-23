#[cfg(feature = "preload")]
pub mod preload;

use crate::{KMSOperatorBackend, keys::SecuredKey};

#[async_trait::async_trait]
pub trait KMSBackend {
    type KeyId;
    type Data;
    type Key: SecuredKey<Self::Data>;
    type Settings;
    type Error;

    fn new(settings: Self::Settings) -> Self;

    fn register(&mut self, key_id: Self::KeyId, key: Self::Key)
    -> Result<Self::KeyId, Self::Error>;

    fn public_key(
        &self,
        key_id: Self::KeyId,
    ) -> Result<<Self::Key as SecuredKey<Self::Data>>::PublicKey, Self::Error>;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: Self::Data,
    ) -> Result<<Self::Key as SecuredKey<Self::Data>>::Signature, Self::Error>;

    async fn execute(
        &mut self,
        key_id: Self::KeyId,
        operator: KMSOperatorBackend<Self>,
    ) -> Result<(), Self::Error>;
}
