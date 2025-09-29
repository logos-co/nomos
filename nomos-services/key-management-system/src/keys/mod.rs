mod ed25519;
mod errors;
mod secured_key;
mod zk;

use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::encodings::{EncodingError, PayloadEncoding, PublicKeyEncoding, SignatureEncoding};
pub use crate::keys::{ed25519::Ed25519Key, errors::KeyError, secured_key::SecuredKey, zk::ZkKey};

/// Entity that gathers all keys provided by the KMS crate.
///
/// Works as a [`SecuredKey`] over [`Encoding`], delegating requests to the
/// appropriate key.
#[expect(dead_code, reason = "Will be used when integrating the KMS service.")]
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub enum Key {
    Ed25519(Ed25519Key),
    Zk(ZkKey),
}

impl SecuredKey for Key {
    type Payload = PayloadEncoding;
    type Signature = SignatureEncoding;
    type PublicKey = PublicKeyEncoding;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        match (self, payload) {
            (Self::Ed25519(key), Self::Payload::Ed25519(payload)) => {
                key.sign(payload).map(Self::Signature::Ed25519)
            }
            (Self::Zk(key), Self::Payload::Zk(payload)) => {
                key.sign(payload).map(Self::Signature::Zk)
            }
            (key, payload) => Err(EncodingError::requires(key, payload).into()),
        }
    }

    fn as_public_key(&self) -> Self::PublicKey {
        match self {
            Self::Ed25519(key) => Self::PublicKey::Ed25519(key.as_public_key()),
            Self::Zk(key) => Self::PublicKey::Zk(key.as_public_key()),
        }
    }
}
