use core::fmt::{self, Debug, Formatter};

use bytes::Bytes;
use ed25519_dalek::{Signature, SigningKey, ed25519::signature::Signer as _};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

mod private;
pub use self::private::{KEY_SIZE as ED25519_SECRET_KEY_SIZE, UnsecuredEd25519Key};
mod public;
pub use self::public::{KEY_SIZE as ED25519_PUBLIC_KEY_SIZE, PublicKey};

/// An hardened Ed25519 secret key that only exposes methods to retrieve public
/// information.
///
/// It is a secured variant of a [`UnsecuredEd25519Key`] and used within the set
/// of supported KMS keys.
#[derive(Deserialize, ZeroizeOnDrop, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "unsafe", derive(Serialize))]
pub struct Ed25519Key(UnsecuredEd25519Key);

impl Ed25519Key {
    #[must_use]
    pub const fn new(signing_key: SigningKey) -> Self {
        Self(UnsecuredEd25519Key(signing_key))
    }
}

impl Debug for Ed25519Key {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "unsafe")]
        write!(f, "Ed25519Key({:?})", self.0)?;

        #[cfg(not(feature = "unsafe"))]
        write!(f, "Ed25519Key(<redacted>)")?;

        Ok(())
    }
}

#[cfg(feature = "unsafe")]
impl From<UnsecuredEd25519Key> for Ed25519Key {
    fn from(value: UnsecuredEd25519Key) -> Self {
        Self(value)
    }
}

impl From<SigningKey> for Ed25519Key {
    fn from(value: SigningKey) -> Self {
        Self(UnsecuredEd25519Key::from(value))
    }
}

#[async_trait::async_trait]
impl SecuredKey for Ed25519Key {
    type Payload = Bytes;
    type Signature = Signature;
    type PublicKey = PublicKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.as_ref().sign(payload.iter().as_slice()))
    }

    fn sign_multiple(
        _keys: &[&Self],
        _payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        unimplemented!("Multi-key signature is not implemented for Ed25519 keys.")
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.public_key()
    }
}
