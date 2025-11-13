use std::marker::PhantomData;

use zeroize::ZeroizeOnDrop;

#[async_trait::async_trait]
pub trait SecureKeyOperations {
    type Key;
    type Error;
    async fn execute(&self, key: &Self::Key) -> Result<(), Self::Error>;
}

pub struct NoKeyOperator<Key, Error> {
    _key: PhantomData<Key>,
    _error: PhantomData<Error>,
}

#[async_trait::async_trait]
impl<Key, Error> SecureKeyOperations for NoKeyOperator<Key, Error>
where
    Key: Send + Sync + 'static,
    Error: Send + Sync + 'static,
{
    type Key = Key;
    type Error = Error;

    async fn execute(&self, key: &Self::Key) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A key that can be used within the Key Management Service.
#[async_trait::async_trait]
pub trait SecuredKey: ZeroizeOnDrop {
    type Payload;
    type Signature;
    type PublicKey;
    type Error;
    type Operations: SecureKeyOperations<Key = Self, Error = Self::Error> + Send + Sync + 'static;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error>;
    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error>
    where
        Self: Sized;
    fn as_public_key(&self) -> Self::PublicKey;

    async fn execute(&self, operator: Self::Operations) -> Result<(), Self::Error> {
        operator.execute(&self).await
    }
}
