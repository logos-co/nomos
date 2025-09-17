mod ed25519;
mod errors;
mod secured_key;

use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::encodings::Encoding;
pub use crate::keys::{
    ed25519::Ed25519Key,
    errors::KeyError,
    secured_key::{SecuredKey, SecuredKeyAdapter},
};

/// Represents a cryptographic key provided by the KMS crate.
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub enum Key {
    Ed25519(Ed25519Key),
}

impl SecuredKey for Key {
    type EncodingFormat = Encoding;
    type Error = KeyError;

    fn sign(&self, data: &Self::EncodingFormat) -> Result<Self::EncodingFormat, Self::Error> {
        match self {
            Self::Ed25519(key) => key.sign_adapted(data),
        }
    }

    fn as_pk(&self) -> Self::EncodingFormat {
        match self {
            Self::Ed25519(key) => key.as_pk(),
        }
        .into()
    }
}
