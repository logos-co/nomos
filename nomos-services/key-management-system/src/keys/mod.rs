mod ed25519;
mod errors;
pub mod secured_key;

pub use crate::keys::{ed25519::Ed25519Key, errors::KeyError};
use crate::{
    encodings::EncodingFormat,
    keys::secured_key::{SecuredKey, SecuredKeyAdapter},
};

/// Represents a cryptographic key provided by the KMS crate.
pub enum Key {
    Ed25519(Ed25519Key),
}

impl SecuredKey for Key {
    type Encoding = EncodingFormat;
    type Error = KeyError;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, Self::Error> {
        match self {
            Key::Ed25519(key) => key.sign_adapted(data),
        }.into()
    }

    fn as_pk(&self) -> Self::Encoding {
        match self {
            Key::Ed25519(key) => key.as_pk(),
        }.into()
    }
}
