use ::serde::{Deserialize, Serialize};
use groth16::Fr;
use nomos_core::crypto::ZkHasher;
use num_bigint::BigUint;

use crate::crypto::proofs::selection::inputs::{ProofOfSelectionInputs, SecretKey};

pub mod inputs;
mod serde;

const DOMAIN_SEPARATION_TAG: [u8; 23] = *b"SELECTION_RANDOMNESS_V1";
pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ProofOfSelection>();

#[derive(Debug)]
pub enum Error {
    InvalidProof,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection {
    #[serde(with = "self::serde::selection_randomness_serde")]
    secret_selection_randomness: Fr,
}

impl ProofOfSelection {
    #[must_use]
    pub fn new(
        ProofOfSelectionInputs {
            ephemeral_key_index,
            secret_key,
            session_number,
        }: ProofOfSelectionInputs,
    ) -> Self {
        let domain_separation_tag: Fr = BigUint::from_bytes_le(&DOMAIN_SEPARATION_TAG[..]).into();
        let ephemeral_key_index: Fr =
            BigUint::from_bytes_le(&ephemeral_key_index.to_le_bytes()[..]).into();
        let session_number: Fr = BigUint::from_bytes_le(&session_number.to_le_bytes()[..]).into();

        let secret_key = match secret_key {
            SecretKey::Core(core_sk) => core_sk,
            // TODO: Change this logic
            SecretKey::PoL(pol_note_sk) => pol_note_sk,
        };
        let secret_selection_randomness = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[
                domain_separation_tag,
                secret_key,
                ephemeral_key_index,
                session_number,
            ]);
            hasher.finalize()
        };

        Self {
            secret_selection_randomness,
        }
    }

    pub fn verify(self, _key_nullifier: &Fr) -> Result<(), Error> {
        todo!("Implement actual verification logic.")
    }

    // TODO: Remove this once the actual proof of selection is implemented.
    #[must_use]
    #[cfg(test)]
    pub const fn dummy() -> Self {
        use groth16::Field as _;

        Self {
            secret_selection_randomness: Fr::ZERO,
        }
    }
}

impl From<Fr> for ProofOfSelection {
    fn from(secret_selection_randomness: Fr) -> Self {
        Self {
            secret_selection_randomness,
        }
    }
}

impl AsRef<Fr> for ProofOfSelection {
    fn as_ref(&self) -> &Fr {
        &self.secret_selection_randomness
    }
}

impl From<[u8; PROOF_OF_SELECTION_SIZE]> for ProofOfSelection {
    fn from(value: [u8; PROOF_OF_SELECTION_SIZE]) -> Self {
        Self {
            secret_selection_randomness: BigUint::from_bytes_be(&value).into(),
        }
    }
}
