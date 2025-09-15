mod ed25519;
mod errors;
pub mod secured_key;
mod zk;

pub use crate::keys::{ed25519::Ed25519Key, errors::KeyError, zk::ZkKey};
use crate::{
    encodings::EncodingFormat,
    keys::secured_key::{SecuredKey, SecuredKeyAdapter},
};

/// Represents a cryptographic key provided by the KMS crate.
pub enum Key {
    Ed25519(Ed25519Key),
    Zk(ZkKey),
}

impl SecuredKey for Key {
    type Encoding = EncodingFormat;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        match self {
            Key::Ed25519(key) => key.sign_adapted(data).map(Self::Encoding::from),
            Key::Zk(key) => key.sign_adapted(data).map(Self::Encoding::from),
        }
    }
}
