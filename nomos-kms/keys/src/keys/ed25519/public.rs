use core::ops::{Deref, DerefMut};

use ed25519_dalek::{PUBLIC_KEY_LENGTH, VerifyingKey};
use nomos_utils::serde::{deserialize_bytes_array, serialize_bytes_array};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

pub const KEY_SIZE: usize = PUBLIC_KEY_LENGTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_bytes_array::<KEY_SIZE, _>(self.0.to_bytes(), serializer)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserialize_bytes_array::<KEY_SIZE, _>(deserializer)?;
        Ok(Self(VerifyingKey::from_bytes(&bytes).map_err(|_| {
            Error::custom("Invalid Ed25519 public key bytes.")
        })?))
    }
}

impl From<VerifyingKey> for PublicKey {
    fn from(value: VerifyingKey) -> Self {
        Self(value)
    }
}

impl From<PublicKey> for VerifyingKey {
    fn from(value: PublicKey) -> Self {
        value.0
    }
}

impl Deref for PublicKey {
    type Target = VerifyingKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PublicKey {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
