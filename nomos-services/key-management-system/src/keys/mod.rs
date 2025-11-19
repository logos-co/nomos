pub mod errors;
pub mod secured_key;

mod ed25519;
mod zk;

use std::pin::pin;

use key_management_system_macros::KmsEnumKey;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

pub use crate::keys::{ed25519::Ed25519Key, zk::ZkKey};
use crate::keys::{
    errors::KeyError,
    secured_key::{BoxedSecureKeyOperations, SecureKeyOperator},
};

/// Entity that gathers all keys provided by the KMS crate.
///
/// Works as a [`SecuredKey`] over [`Encoding`], delegating requests to the
/// appropriate key.
#[derive(Serialize, Deserialize, ZeroizeOnDrop, PartialEq, Eq, Clone, Debug, KmsEnumKey)]
pub enum Key {
    Ed25519(Ed25519Key),
    Zk(ZkKey),
}

pub enum KeyOperators {
    Ed25519(BoxedSecureKeyOperations<Ed25519Key>),
    Zk(BoxedSecureKeyOperations<ZkKey>),
}

#[async_trait::async_trait]
impl SecureKeyOperator for KeyOperators {
    type Key = Key;
    type Error = KeyError;

    async fn execute(&mut self, key: &Self::Key) -> Result<(), Self::Error> {
        match (self, key) {
            (KeyOperators::Ed25519(operator), Key::Ed25519(key)) => operator.execute(key).await,
            (KeyOperators::Zk(operator), Key::Zk(key)) => operator.execute(key).await,
            (_operator, key) => Err(KeyError::UnsupportedOperation(format!("{key:?}"))),
        }
    }
}

impl From<Ed25519Key> for Key {
    fn from(value: Ed25519Key) -> Self {
        Self::Ed25519(value)
    }
}

impl From<ZkKey> for Key {
    fn from(value: ZkKey) -> Self {
        Self::Zk(value)
    }
}
