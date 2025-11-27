use blake2::{Blake2b512, digest::Digest as _};
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
    // FIXME: This function must accept `rng` as an argument.
    let mut rng = BlakeRng::from_seed(blake2b512(&[key]).into());

    let mut buf = [0u8; SIZE];
    blake_random_bytes(&mut buf, &mut rng);
    buf
}

/// Generates pseudo-random bytes of the given size
/// using [`BlakeRng`] cipher with a key derived from the input key.
#[must_use]
pub fn pseudo_random_bytes(size: usize, rng: &mut BlakeRng) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    blake_random_bytes(&mut buf, rng);
    buf
}

fn blake_random_bytes(buf: &mut [u8], rng: &mut BlakeRng) {
    rng.fill_bytes(buf);
}

pub(crate) fn blake2b512(inputs: &[&[u8]]) -> [u8; 64] {
    let mut hasher = Blake2b512::new();
    for input in inputs {
        hasher.update(input);
    }
    hasher.finalize().into()
}
