use std::{fmt::Debug, marker::PhantomData};

use zeroize::ZeroizeOnDrop;

#[async_trait::async_trait]
pub trait SecureKeyOperator {
    type Key;
    type Error;
    async fn execute(&mut self, key: &Self::Key) -> Result<(), Self::Error>;
}

pub trait DebugSecureKeyOperator: SecureKeyOperator + Debug {}
impl<T: SecureKeyOperator + Debug> DebugSecureKeyOperator for T {}

pub type BoxedSecureKeyOperator<Key> =
    Box<dyn DebugSecureKeyOperator<Key = Key, Error = <Key as SecuredKey>::Error> + Send + Sync>;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct NoKeyOperator<Key, Error> {
    _key: PhantomData<Key>,
    _error: PhantomData<Error>,
}

#[async_trait::async_trait]
impl<Key, Error> SecureKeyOperator for NoKeyOperator<Key, Error>
where
    Key: Send + Sync + 'static,
    Error: Send + Sync + 'static,
{
    type Key = Key;
    type Error = Error;

    async fn execute(&mut self, _key: &Self::Key) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<Key, Error> Default for NoKeyOperator<Key, Error> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Key, Error> NoKeyOperator<Key, Error> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _key: PhantomData,
            _error: PhantomData,
        }
    }
}

/// A key that can be used within the Key Management Service.
#[async_trait::async_trait]
pub trait SecuredKey: ZeroizeOnDrop {
    type Payload;
    type Signature;
    type PublicKey;
    type Error;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error>;
    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error>
    where
        Self: Sized;
    fn as_public_key(&self) -> Self::PublicKey;

    async fn execute<Operation>(&self, mut operator: Operation) -> Result<(), Self::Error>
    where
        Operation: SecureKeyOperator<Key = Self, Error = Self::Error> + Send + Debug,
    {
        operator.execute(self).await
    }
}
