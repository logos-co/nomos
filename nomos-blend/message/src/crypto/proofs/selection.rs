use serde::{Deserialize, Serialize};

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
