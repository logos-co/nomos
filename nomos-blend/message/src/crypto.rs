use blake2::{
    digest::{Update as _, VariableOutput as _},
    Blake2bVar,
};
use chacha20::ChaCha20;
use cipher::{KeyIvInit as _, StreamCipher as _};
use ed25519_dalek::{ed25519::signature::Signer as _, Verifier as _};
use rand_chacha::{
    rand_core::{RngCore as _, SeedableRng as _},
    ChaCha12Rng, ChaCha20Rng,
};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

pub const KEY_SIZE: usize = 32;

#[derive(Clone)]
pub struct Ed25519PrivateKey(ed25519_dalek::SigningKey);

impl Ed25519PrivateKey {
    /// Generates a new Ed25519 private key using the [`ChaCha12Rng`].
    #[must_use]
    pub fn generate() -> Self {
        Self(ed25519_dalek::SigningKey::generate(
            &mut ChaCha12Rng::from_entropy(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        self.0.verify(body, &signature.0).is_ok()
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

pub const SIGNATURE_SIZE: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature(ed25519_dalek::Signature);

impl From<ed25519_dalek::Signature> for Signature {
    fn from(sig: ed25519_dalek::Signature) -> Self {
        Self(sig)
    }
}

impl From<[u8; SIGNATURE_SIZE]> for Signature {
    fn from(bytes: [u8; SIGNATURE_SIZE]) -> Self {
        ed25519_dalek::Signature::from_bytes(&bytes).into()
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

pub const PROOF_OF_QUOTA_SIZE: usize = 160;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfQuota(#[serde(with = "BigArray")] [u8; PROOF_OF_QUOTA_SIZE]);

impl ProofOfQuota {
    // TODO: Remove this once the actual proof of quota is implemented.
    #[must_use]
    pub const fn dummy() -> Self {
        Self([6u8; PROOF_OF_QUOTA_SIZE])
    }
}

impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        Self(bytes)
    }
}

pub const PROOF_OF_SELECTION_SIZE: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection([u8; PROOF_OF_SELECTION_SIZE]);

impl ProofOfSelection {
    // TODO: Implement actual verification logic.
    #[must_use]
    pub fn verify(&self) -> bool {
        self == &Self::dummy()
    }

    // TODO: Remove this once the actual proof of selection is implemented.
    #[must_use]
    pub const fn dummy() -> Self {
        Self([7u8; PROOF_OF_SELECTION_SIZE])
    }
}

impl From<[u8; PROOF_OF_SELECTION_SIZE]> for ProofOfSelection {
    fn from(bytes: [u8; PROOF_OF_SELECTION_SIZE]) -> Self {
        Self(bytes)
    }
}

/// Generates random bytes of the constant size using [`ChaCha20`].
#[must_use]
pub fn random_sized_bytes<const SIZE: usize>() -> [u8; SIZE] {
    let mut buf = [0u8; SIZE];
    ChaCha20Rng::from_entropy().fill_bytes(&mut buf);
    buf
}

/// Generates pseudo-random bytes of the constant size
/// using [`ChaCha20`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_sized_bytes<const SIZE: usize>(key: &[u8]) -> [u8; SIZE] {
    let mut buf = [0u8; SIZE];
    chacha20_random_bytes(&mut buf, key);
    buf
}

/// Generates pseudo-random bytes of the given size
/// using [`ChaCha20`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_bytes(key: &[u8], size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    chacha20_random_bytes(&mut buf, key);
    buf
}

fn chacha20_random_bytes(buf: &mut [u8], key: &[u8]) {
    const IV: [u8; 12] = [0u8; 12];
    let mut cipher = ChaCha20::new_from_slices(&blake2b256(key), &IV).unwrap();
    cipher.apply_keystream(buf);
}

const HASH_SIZE: usize = 32; // Size of the hash output for Blake2b-256

fn blake2b256(input: &[u8]) -> [u8; HASH_SIZE] {
    let mut hasher = Blake2bVar::new(HASH_SIZE).unwrap();
    hasher.update(input);
    let mut output = [0u8; HASH_SIZE];
    hasher.finalize_variable(&mut output).unwrap();
    output
}
