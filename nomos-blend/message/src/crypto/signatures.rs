use serde::{Deserialize, Serialize};

pub const SIGNATURE_SIZE: usize = size_of::<ed25519_dalek::Signature>();

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

impl AsRef<ed25519_dalek::Signature> for Signature {
    fn as_ref(&self) -> &ed25519_dalek::Signature {
        &self.0
    }
}
