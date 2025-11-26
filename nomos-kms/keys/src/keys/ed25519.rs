use bytes::Bytes;
use ed25519_dalek::{
    SECRET_KEY_LENGTH, Signature, SigningKey, VerifyingKey, ed25519::signature::Signer as _,
};
use nomos_utils::serde::{deserialize_bytes_array, serialize_bytes_array};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

pub const KEY_SIZE: usize = SECRET_KEY_LENGTH;

#[derive(PartialEq, Eq, Clone, Debug, ZeroizeOnDrop)]
pub struct Ed25519Key(SigningKey);

impl Serialize for Ed25519Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_bytes_array::<KEY_SIZE, _>(self.0.to_bytes(), serializer)
    }
}

impl<'de> Deserialize<'de> for Ed25519Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserialize_bytes_array::<KEY_SIZE, _>(deserializer)?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }
}

impl Ed25519Key {
    /// Generates a new Ed25519 private key using the [`BlakeRng`].
    #[must_use]
    pub fn generate<Rng>(rng: &mut Rng) -> Self
    where
        Rng: rand::CryptoRng + rand::RngCore,
    {
        Self(SigningKey::generate(rng))
    }

    #[must_use]
    pub const fn new(signing_key: SigningKey) -> Self {
        Self(signing_key)
    }

    /// Signs a message.
    #[must_use]
    pub fn sign_payload(&self, message: &[u8]) -> Signature {
        self.0.sign(message)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn public_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }
}

impl From<[u8; KEY_SIZE]> for Ed25519Key {
    fn from(bytes: [u8; KEY_SIZE]) -> Self {
        Self(SigningKey::from_bytes(&bytes))
    }
}

impl From<SigningKey> for Ed25519Key {
    fn from(value: SigningKey) -> Self {
        Self(value)
    }
}

impl From<Ed25519Key> for SigningKey {
    fn from(value: Ed25519Key) -> Self {
        value.0.clone()
    }
}

impl AsRef<SigningKey> for Ed25519Key {
    fn as_ref(&self) -> &SigningKey {
        &self.0
    }
}

#[async_trait::async_trait]
impl SecuredKey for Ed25519Key {
    type Payload = Bytes;
    type Signature = Signature;
    type PublicKey = VerifyingKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload.iter().as_slice()))
    }

    fn sign_multiple(
        _keys: &[&Self],
        _payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        unimplemented!("Multi-key signature is not implemented for Ed25519 keys.")
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.verifying_key()
    }
}
