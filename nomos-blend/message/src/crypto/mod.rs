use blake2::{digest::Digest as _, Blake2b512};
use nomos_utils::blake_rng::{BlakeRng, RngCore as _, SeedableRng as _};

pub mod keys;
pub mod proofs;
pub mod signatures;

/// Generates random bytes of the constant size using [`BlakeRng`].
#[must_use]
pub fn random_sized_bytes<const SIZE: usize>() -> [u8; SIZE] {
    let mut buf = [0u8; SIZE];
    BlakeRng::from_entropy().fill_bytes(&mut buf);
    buf
}

/// Generates pseudo-random bytes of the constant size
/// using [`BlakeRng`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_sized_bytes<const SIZE: usize>(key: &[u8]) -> [u8; SIZE] {
    let mut buf = [0u8; SIZE];
    blake_random_bytes(&mut buf, key);
    buf
}

/// Generates pseudo-random bytes of the given size
/// using [`BlakeRng`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_bytes(key: &[u8], size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    blake_random_bytes(&mut buf, key);
    buf
}

fn blake_random_bytes(buf: &mut [u8], key: &[u8]) {
    let mut cipher = BlakeRng::from_seed(blake2b512(key).into());
    cipher.fill_bytes(buf);
}

fn blake2b512(input: &[u8]) -> [u8; 64] {
    let mut hasher = Blake2b512::new();
    hasher.update(input);
    hasher.finalize().into()
}
