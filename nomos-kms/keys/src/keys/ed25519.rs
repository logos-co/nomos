use core::fmt::{self, Debug, Formatter};

use bytes::Bytes;
use ed25519_dalek::{
    SECRET_KEY_LENGTH, Signature, SigningKey, VerifyingKey, ed25519::signature::Signer as _,
};
use nomos_utils::serde::{deserialize_bytes_array, serialize_bytes_array};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq as _;
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

pub const KEY_SIZE: usize = SECRET_KEY_LENGTH;

/// An Ed25519 secret key exposing methods to retrieve its inner secret value.
///
/// To be used in contexts where a KMS-like key is required, but it's not
/// possible to go through the KMS roundtrip of executing operators.
#[derive(ZeroizeOnDrop, Clone)]
pub struct UnsecuredEd25519Key(SigningKey);

impl Serialize for UnsecuredEd25519Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_bytes_array::<KEY_SIZE, _>(self.0.to_bytes(), serializer)
    }
}

impl<'de> Deserialize<'de> for UnsecuredEd25519Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserialize_bytes_array::<KEY_SIZE, _>(deserializer)?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }
}

impl PartialEq for UnsecuredEd25519Key {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes().ct_eq(other.0.as_bytes()).into()
    }
}

impl Eq for UnsecuredEd25519Key {}

impl From<SigningKey> for UnsecuredEd25519Key {
    fn from(value: SigningKey) -> Self {
        Self(value)
    }
}

impl From<UnsecuredEd25519Key> for SigningKey {
    fn from(value: UnsecuredEd25519Key) -> Self {
        value.0.clone()
    }
}

/// Return a reference to the inner secret key.
impl AsRef<SigningKey> for UnsecuredEd25519Key {
    fn as_ref(&self) -> &SigningKey {
        &self.0
    }
}

/// An hardened Ed25519 secret key that only exposes methods to retrieve public
/// information.
///
/// It is a secured variant of a [`Ed25519Key`] and used within the set of
/// supported KMS keys.
#[derive(Deserialize, ZeroizeOnDrop, Clone)]
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
        write!(f, "Ed25519Key(<redacted>)")
    }
}

impl PartialEq for Ed25519Key {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Ed25519Key {}

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
    type PublicKey = VerifyingKey;
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
        self.0.as_ref().verifying_key()
    }
}
