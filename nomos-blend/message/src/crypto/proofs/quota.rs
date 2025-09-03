use serde::{Deserialize, Serialize};

pub const PROOF_OF_QUOTA_SIZE: usize = 160;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ProofOfQuota(#[serde(with = "serde_big_array::BigArray")] [u8; PROOF_OF_QUOTA_SIZE]);

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
