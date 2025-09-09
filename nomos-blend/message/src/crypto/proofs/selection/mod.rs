use groth16::{serde::serde_fr, Fr};
use nomos_core::crypto::{ZkHash, ZkHasher};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::crypto::proofs::selection::inputs::{ProofOfSelectionInputs, SecretKey};

pub mod inputs;

const DOMAIN_SEPARATION_TAG: [u8; 23] = *b"SELECTION_RANDOMNESS_V1";
pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ProofOfSelection>();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection {
    #[serde(with = "serde_fr")]
    secret_selection_randomness: ZkHash,
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
            SecretKey::PoL(_) => ZkHash::default(),
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

    // TODO: Implement actual verification logic.
    #[must_use]
    pub fn verify(self, key_nullifier: &ZkHash) -> Result<(), ()> {
        let hashed_secret_randomness = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[self.secret_selection_randomness]);
            hasher.finalize()
        };
        // TODO: Remove check with dummy
        if &hashed_secret_randomness == key_nullifier || self == Self::dummy() {
            Ok(())
        } else {
            Err(())
        }
    }

    // TODO: Remove this once the actual proof of selection is implemented.
    #[must_use]
    pub fn dummy() -> Self {
        Self {
            secret_selection_randomness: ZkHash::default(),
        }
    }
}

impl From<ZkHash> for ProofOfSelection {
    fn from(hash: ZkHash) -> Self {
        Self {
            secret_selection_randomness: hash,
        }
    }
}

impl AsRef<ZkHash> for ProofOfSelection {
    fn as_ref(&self) -> &ZkHash {
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
