mod ed25519;
mod errors;
mod secured_key;

use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::encodings::{PayloadEncoding, PublicKeyEncoding, SignatureEncoding};
pub use crate::keys::{ed25519::Ed25519Key, errors::KeyError, secured_key::SecuredKey};

/// Entity that gathers all keys provided by the KMS crate.
///
/// Works as a [`SecuredKey`] over [`Encoding`], delegating requests to the
/// appropriate key.
#[expect(dead_code, reason = "Will be used when integrating the KMS service.")]
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub enum Key {
    Ed25519(Ed25519Key),
}

impl SecuredKey for Key {
    type Payload = PayloadEncoding;
    type Signature = SignatureEncoding;
    type PublicKey = PublicKeyEncoding;
    type Error = KeyError;

    fn sign(&self, data: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        match (self, data) {
            (Self::Ed25519(key), Self::Payload::Ed25519(data)) => {
                key.sign(data).map(Self::Signature::Ed25519)
            }
        }
    }

    fn as_public_key(&self) -> Self::PublicKey {
        match self {
            Self::Ed25519(key) => Self::PublicKey::Ed25519(key.as_public_key()),
        }
    }
}
