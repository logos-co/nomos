mod ed25519;
mod errors;
mod secured_key;

use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::encodings::Encoding;
pub use crate::keys::{ed25519::Ed25519Key, errors::KeyError, secured_key::SecuredKey};

/// A cryptographic key dispatcher that covers all keys provided by the KMS
/// crate.
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub enum Key {
    Ed25519(Ed25519Key),
}

impl SecuredKey<Encoding> for Key {
    type Signature = Encoding;
    type PublicKey = Encoding;
    type Error = KeyError;

    fn sign(&self, data: &Encoding) -> Result<Self::Signature, Self::Error> {
        match self {
            Self::Ed25519(key) => key.sign(data),
        }
    }

    fn as_public_key(&self) -> Self::PublicKey {
        match self {
            Self::Ed25519(key) => <Ed25519Key as SecuredKey<Encoding>>::as_public_key(key),
        }
    }
}
