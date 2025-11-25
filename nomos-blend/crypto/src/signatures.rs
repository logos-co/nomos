use core::hash::{Hash, Hasher};

use ed25519_dalek::SIGNATURE_LENGTH;
use serde::{Deserialize, Serialize};

pub const ED25519_SIGNATURE_SIZE: usize = SIGNATURE_LENGTH;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature(ed25519_dalek::Signature);

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

impl From<[u8; ED25519_SIGNATURE_SIZE]> for Signature {
    fn from(bytes: [u8; ED25519_SIGNATURE_SIZE]) -> Self {
        ed25519_dalek::Signature::from_bytes(&bytes).into()
    }
}

impl AsRef<ed25519_dalek::Signature> for Signature {
    fn as_ref(&self) -> &ed25519_dalek::Signature {
        &self.0
    }
}
