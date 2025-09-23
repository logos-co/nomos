use ed25519_dalek::{Verifier as _, ed25519::signature::Signer as _};
use nomos_utils::blake_rng::{BlakeRng, SeedableRng as _};
use serde::{Deserialize, Serialize};

use crate::crypto::{pseudo_random_bytes, signatures::Signature};

pub const KEY_SIZE: usize = 32;

#[derive(Clone)]
pub struct Ed25519PrivateKey(ed25519_dalek::SigningKey);

impl Ed25519PrivateKey {
    /// Generates a new Ed25519 private key using the [`BlakeRng`].
    #[must_use]
    pub fn generate() -> Self {
        Self(ed25519_dalek::SigningKey::generate(
            &mut BlakeRng::from_entropy(),
        ))
    }

    #[must_use]
    pub fn public_key(&self) -> Ed25519PublicKey {
        Ed25519PublicKey(self.0.verifying_key())
    }

    /// Signs a message.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.0.sign(message).into()
    }

    /// Derives an X25519 private key.
    #[must_use]
    pub fn derive_x25519(&self) -> X25519PrivateKey {
        self.0.to_scalar_bytes().into()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        self.0.as_bytes()
    }
}

impl From<[u8; KEY_SIZE]> for Ed25519PrivateKey {
    fn from(bytes: [u8; KEY_SIZE]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(&bytes))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Ed25519PublicKey(ed25519_dalek::VerifyingKey);

impl Ed25519PublicKey {
    /// Derives an X25519 public key.
    #[must_use]
    pub fn derive_x25519(&self) -> X25519PublicKey {
        self.0.to_montgomery().to_bytes().into()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn verify_signature(&self, body: &[u8], signature: &Signature) -> bool {
        self.0.verify(body, signature.as_ref()).is_ok()
    }
}

impl From<Ed25519PublicKey> for [u8; KEY_SIZE] {
    fn from(key: Ed25519PublicKey) -> Self {
        key.0.to_bytes()
    }
}

impl TryFrom<[u8; KEY_SIZE]> for Ed25519PublicKey {
    type Error = String;

    fn try_from(key: [u8; KEY_SIZE]) -> Result<Self, Self::Error> {
        Ok(Self(
            ed25519_dalek::VerifyingKey::from_bytes(&key)
                .map_err(|_| "Invalid Ed25519 public key".to_owned())?,
        ))
    }
}

#[derive(Clone)]
pub struct X25519PrivateKey(x25519_dalek::StaticSecret);

impl X25519PrivateKey {
    #[must_use]
    pub fn derive_shared_key(&self, public_key: &X25519PublicKey) -> SharedKey {
        SharedKey(self.0.diffie_hellman(&public_key.0).to_bytes())
    }
}

impl From<[u8; KEY_SIZE]> for X25519PrivateKey {
    fn from(bytes: [u8; KEY_SIZE]) -> Self {
        Self(x25519_dalek::StaticSecret::from(bytes))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct X25519PublicKey(x25519_dalek::PublicKey);

impl From<[u8; KEY_SIZE]> for X25519PublicKey {
    fn from(bytes: [u8; KEY_SIZE]) -> Self {
        Self(x25519_dalek::PublicKey::from(bytes))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SharedKey([u8; KEY_SIZE]);

impl SharedKey {
    #[must_use]
    pub const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Encrypts data in-place by XOR operation with a pseudo-random bytes.
    pub fn encrypt(&self, data: &mut [u8]) {
        Self::xor_in_place(data, &pseudo_random_bytes(self.as_slice(), data.len()));
    }

    /// Decrypts data in-place by XOR operation with a pseudo-random bytes.
    pub fn decrypt(&self, data: &mut [u8]) {
        self.encrypt(data); // encryption and decryption are symmetric.
    }

    fn xor_in_place(a: &mut [u8], b: &[u8]) {
        assert_eq!(a.len(), b.len());
        a.iter_mut().zip(b.iter()).for_each(|(x1, &x2)| *x1 ^= x2);
    }
}
