use core::{
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
};

use ed25519_dalek::SIGNATURE_LENGTH;
use serde::{Deserialize, Serialize};

pub const SIGNATURE_SIZE: usize = SIGNATURE_LENGTH;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature(ed25519_dalek::Signature);

impl Signature {
    #[must_use]
    pub fn from_bytes(bytes: &[u8; SIGNATURE_SIZE]) -> Self {
        Self(ed25519_dalek::Signature::from_bytes(bytes))
    }
}

impl Hash for Signature {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.0.to_bytes().hash(state);
    }
}

impl From<ed25519_dalek::Signature> for Signature {
    fn from(sig: ed25519_dalek::Signature) -> Self {
        Self(sig)
    }
}

impl From<Signature> for ed25519_dalek::Signature {
    fn from(sig: Signature) -> Self {
        sig.0
    }
}

impl From<[u8; SIGNATURE_SIZE]> for Signature {
    fn from(bytes: [u8; SIGNATURE_SIZE]) -> Self {
        Self::from_bytes(&bytes)
    }
}

impl Deref for Signature {
    type Target = ed25519_dalek::Signature;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Signature {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
