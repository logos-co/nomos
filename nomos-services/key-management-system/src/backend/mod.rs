#[cfg(feature = "preload")]
pub mod preload;

use crate::{KMSOperatorBackend, keys::secured_key::SecuredKey};

#[async_trait::async_trait]
pub trait KMSBackend {
    type KeyId;
    type Key: SecuredKey;
    type Settings;
    type Error;

    fn new(settings: Self::Settings) -> Self;

    fn register(&mut self, key_id: Self::KeyId, key: Self::Key)
    -> Result<Self::KeyId, Self::Error>;

    fn public_key(
        &self,
        key_id: Self::KeyId,
    ) -> Result<<Self::Key as SecuredKey>::PublicKey, Self::Error>;

    fn sign(
        &self,
        key_id: Self::KeyId,
        payload: <Self::Key as SecuredKey>::Payload,
    ) -> Result<<Self::Key as SecuredKey>::Signature, Self::Error>;

    fn sign_multiple(
        &self,
        key_ids: Vec<Self::KeyId>,
        payload: <Self::Key as SecuredKey>::Payload,
    ) -> Result<<Self::Key as SecuredKey>::Signature, Self::Error>;

    async fn execute(
        &mut self,
        key_id: Self::KeyId,
        operator: KMSOperatorBackend<Self>,
    ) -> Result<(), Self::Error>;
}
