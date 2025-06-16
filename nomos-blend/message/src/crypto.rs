use blake2::{
    digest::{Update as _, VariableOutput as _},
    Blake2bVar,
};
use chacha20::ChaCha20;
use cipher::{KeyIvInit as _, StreamCipher as _};
use ed25519_dalek::ed25519::signature::Signer as _;
use rand_chacha::{rand_core::SeedableRng as _, ChaCha12Rng};

pub const KEY_SIZE: usize = 32;
pub const SIGNATURE_SIZE: usize = 64;

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
    pub fn from_bytes(bytes: &[u8; KEY_SIZE]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn public_key(&self) -> Ed25519PublicKey {
        Ed25519PublicKey(self.0.verifying_key())
    }

    /// Signs a message.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_SIZE] {
        self.0.sign(message).to_bytes()
    }

    /// Derives an X25519 private key.
    #[must_use]
    pub fn derive_x25519(&self) -> X25519PrivateKey {
        X25519PrivateKey::from_bytes(self.0.to_scalar_bytes())
    }
}

#[derive(Clone)]
pub struct Ed25519PublicKey(ed25519_dalek::VerifyingKey);

impl Ed25519PublicKey {
    pub fn from_bytes(bytes: &[u8; KEY_SIZE]) -> Result<Self, String> {
        ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map_err(|e| e.to_string())
            .map(Self)
    }

    pub fn from_slice(slice: &[u8]) -> Result<Self, String> {
        if slice.len() != KEY_SIZE {
            return Err(format!(
                "Invalid key size: expected {}, got {}",
                KEY_SIZE,
                slice.len()
            ));
        }
        Self::from_bytes(slice.try_into().unwrap())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        self.0.as_bytes()
    }

    /// Derives an X25519 public key.
    #[must_use]
    pub fn derive_x25519(&self) -> X25519PublicKey {
        X25519PublicKey::from_bytes(self.0.to_montgomery().to_bytes())
    }
}

#[derive(Clone)]
pub struct X25519PrivateKey(x25519_dalek::StaticSecret);

impl X25519PrivateKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Self {
        Self(x25519_dalek::StaticSecret::from(bytes))
    }

    #[must_use]
    pub fn public_key(&self) -> X25519PublicKey {
        X25519PublicKey(x25519_dalek::PublicKey::from(&self.0))
    }

    #[must_use]
    pub fn derive_shared_key(&self, public_key: &X25519PublicKey) -> SharedKey {
        SharedKey(self.0.diffie_hellman(&public_key.0).to_bytes())
    }
}

#[derive(Clone)]
pub struct X25519PublicKey(x25519_dalek::PublicKey);

impl X25519PublicKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Self {
        Self(x25519_dalek::PublicKey::from(bytes))
    }
}

#[derive(Clone)]
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

#[derive(Clone)]
pub struct ProofOfQuota(pub [u8; PROOF_OF_QUOTA_SIZE]);

impl ProofOfQuota {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PROOF_OF_QUOTA_SIZE] {
        &self.0
    }
}

pub const PROOF_OF_SELECTION_SIZE: usize = 32;

#[derive(Clone)]
pub struct ProofOfSelection(pub [u8; PROOF_OF_SELECTION_SIZE]);

impl ProofOfSelection {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PROOF_OF_SELECTION_SIZE] {
        &self.0
    }
}

/// Generates pseudo-random bytes of the given size
/// using [`ChaCha20`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_bytes(key: &[u8], size: usize) -> Vec<u8> {
    const IV: [u8; 12] = [0u8; 12];
    let mut cipher = ChaCha20::new_from_slices(&blake2b256(key), &IV).unwrap();
    let mut buf = vec![0u8; size];
    cipher.apply_keystream(&mut buf);
    buf
}

const HASH_SIZE: usize = 32; // Size of the hash output for Blake2b-256

fn blake2b256(input: &[u8]) -> [u8; HASH_SIZE] {
    let mut hasher = Blake2bVar::new(HASH_SIZE).unwrap();
    hasher.update(input);
    let mut output = [0u8; HASH_SIZE];
    hasher.finalize_variable(&mut output).unwrap();
    output
}
