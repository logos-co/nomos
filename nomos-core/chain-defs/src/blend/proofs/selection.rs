use groth16::{fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use poseidon2::ZkHash;
use serde::{Deserialize, Serialize};

pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ZkHash>();

#[derive(Clone, Debug, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProofOfSelection {
    #[serde(with = "groth16::serde::serde_fr")]
    pub selection_randomness: ZkHash,
}

impl ProofOfSelection {
    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_SELECTION_SIZE]) -> Self {
        Self {
            selection_randomness: fr_from_bytes_unchecked(&bytes),
        }
    }
}

impl TryFrom<[u8; PROOF_OF_SELECTION_SIZE]> for ProofOfSelection {
    type Error = Box<dyn std::error::Error>;

    fn try_from(value: [u8; PROOF_OF_SELECTION_SIZE]) -> Result<Self, Self::Error> {
        Ok(Self {
            selection_randomness: fr_from_bytes(&value).map_err(Box::new)?,
        })
    }
}

impl From<&ProofOfSelection> for [u8; PROOF_OF_SELECTION_SIZE] {
    fn from(proof: &ProofOfSelection) -> Self {
        fr_to_bytes(&proof.selection_randomness)
    }
}
