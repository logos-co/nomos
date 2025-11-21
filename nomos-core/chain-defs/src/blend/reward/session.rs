use std::ops::Deref;

use blake2::{Blake2b512, Digest as _};
use groth16::fr_to_bytes;
use poseidon2::ZkHash;

use crate::sdp::SessionNumber;

/// Deterministic unbiased randomness for a session.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SessionRandomness([u8; 64]);

impl Deref for SessionRandomness {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<[u8; 64]> for SessionRandomness {
    fn from(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }
}

const SESSION_RANDOMNESS_TAG: [u8; 27] = *b"BLEND_SESSION_RANDOMNESS_V1";

impl SessionRandomness {
    /// Derive the session randomness from the given session number and epoch
    /// nonce.
    #[must_use]
    pub fn new(session_number: SessionNumber, epoch_nonce: &ZkHash) -> Self {
        Self(blake2b512(&[
            &SESSION_RANDOMNESS_TAG,
            &fr_to_bytes(epoch_nonce),
            &session_number.to_le_bytes(),
        ]))
    }
}

fn blake2b512(inputs: &[&[u8]]) -> [u8; 64] {
    let mut hasher = Blake2b512::new();
    for input in inputs {
        hasher.update(input);
    }
    hasher.finalize().into()
}
