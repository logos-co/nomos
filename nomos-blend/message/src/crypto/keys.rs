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

impl From<ed25519_dalek::SigningKey> for Ed25519PrivateKey {
    fn from(value: ed25519_dalek::SigningKey) -> Self {
        Self(value)
    }
}

impl From<Ed25519PrivateKey> for ed25519_dalek::SigningKey {
    fn from(value: Ed25519PrivateKey) -> Self {
        Self::from_bytes(value.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_key_encryption_security() {
        let plain1 = b"hello".to_vec();
        println!("plain1: {plain1:?}");
        let plain2 = b"world".to_vec();
        println!("plain2: {plain2:?}");

        // Encrypt two different data using the same key.
        // A new PRG is created with the same seed (key) each time.
        let key = SharedKey([0; _]);
        let cipher1 = encrypt_cloned(&plain1, &key);
        let cipher2 = encrypt_cloned(&plain2, &key);
        println!("cipher2: {cipher2:?}");

        // XOR the two ciphertexts
        let xor_two_ciphers = xor(&cipher1, &cipher2);
        println!("XOR(cipher1, cipher2): {xor_two_ciphers:?}");

        // XOR the two plaintexts
        let xor_two_plains = xor(&plain1, &plain2);
        println!("XOR(plain1, plain2): {xor_two_plains:?}");

        // xor_two_plains and xor_two_ciphers are the same
        // because `encrypt` creates a new PRG with the same seed (key) each time.
        assert_eq!(xor_two_plains, xor_two_ciphers);

        // Because of that, plain2 can be recovered as below,
        // even if he doesn't know the key, but knows plain "somehow".
        let leaked_plain2 = xor(&plain1, &xor_two_ciphers);
        assert_eq!(leaked_plain2, plain2);
    }

    #[test]
    fn shared_key_encapsulation_security() {
        let plain1 = b"hello".to_vec();
        println!("plain1: {plain1:?}");
        let plain2 = b"world".to_vec();
        println!("plain2: {plain2:?}");

        let key1 = SharedKey([0; _]);
        let key2 = SharedKey([1; _]);

        // First, encrypt the plain2 with the key2.
        let cipher2 = encrypt_cloned(&plain2, &key2);

        // Second, encrypt plain1 and cipher2 with key1.
        let cipher1 = encrypt_cloned(&plain1, &key1);
        let double_cipher2 = encrypt_cloned(&cipher2, &key1);

        // XOR cipher1 and double_cipher2
        let xor_cipher1_and_double_cipher2 = xor(&cipher1, &double_cipher2);
        println!("XOR(cipher1, double_cipher2): {xor_cipher1_and_double_cipher2:?}");

        // Now, someone who knows the key1 can recover plain1 and cipher2 (not plain2).
        // This is the intended use case.
        let recovered_plain1 = decrypt_cloned(&cipher1, &key1);
        assert_eq!(recovered_plain1, plain1);
        let recovered_cipher2 = decrypt_cloned(&double_cipher2, &key1);
        assert_eq!(recovered_cipher2, cipher2);

        // If someone, who doesn't know the key1, knows the plain1 "somehow",
        // he can recover cipher2 as below.
        // It's because `encrypt` creates a new PRG with the same seed (key) each time.
        let leaked_cipher2 = xor(&plain1, &xor_cipher1_and_double_cipher2);
        assert_eq!(leaked_cipher2, cipher2);
        // Even if he recovers cipher2, he can't decrypt it unless he knows key2.
        // But, what if he is the one who has key2?
    }

    fn encrypt_cloned(data: &[u8], key: &SharedKey) -> Vec<u8> {
        let mut buf = data.to_vec();
        key.encrypt(&mut buf);
        buf
    }

    fn decrypt_cloned(data: &[u8], key: &SharedKey) -> Vec<u8> {
        let mut buf = data.to_vec();
        key.decrypt(&mut buf);
        buf
    }

    fn xor(a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut buf = a.to_vec();
        SharedKey::xor_in_place(&mut buf, b);
        buf
    }
}
