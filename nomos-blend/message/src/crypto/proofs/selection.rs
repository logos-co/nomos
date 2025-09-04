use groth16::{serde::serde_fr, Fr};
use nomos_core::crypto::{ZkHash, ZkHasher};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

pub const PROOF_OF_SELECTION_SIZE: usize = 32;

const DOMAIN_SEPARATION_TAG: [u8; 23] = *b"SELECTION_RANDOMNESS_V1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection(#[serde(with = "serde_fr")] ZkHash);

pub struct ProofOfSelectionInputs {
    pub secret_key: ZkHash,
    pub ephemeral_key_index: usize,
    pub session_number: u64,
}

impl ProofOfSelection {
    // TODO: Implement actual verification logic.
    #[must_use]
    pub fn verify(self, key_nullifier: &ZkHash) -> bool {
        let hashed_secret_randomness = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[self.0]);
            hasher.finalize()
        };
        // TODO: Remove check with dummy
        &hashed_secret_randomness == key_nullifier || self == Self::dummy()
    }

    // TODO: Remove this once the actual proof of selection is implemented.
    #[must_use]
    pub fn dummy() -> Self {
        Self(ZkHash::default())
    }

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

        let hash = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[
                domain_separation_tag,
                secret_key,
                ephemeral_key_index,
                session_number,
            ]);
            hasher.finalize()
        };

        Self(hash)
    }
}

impl From<ZkHash> for ProofOfSelection {
    fn from(hash: ZkHash) -> Self {
        Self(hash)
    }
}

impl AsRef<ZkHash> for ProofOfSelection {
    fn as_ref(&self) -> &ZkHash {
        &self.0
    }
}
