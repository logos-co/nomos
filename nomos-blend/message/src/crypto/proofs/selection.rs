use groth16::{serde::serde_fr, Fr};
use nomos_core::crypto::{ZkHash, ZkHasher};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

pub const PROOF_OF_SELECTION_SIZE: usize = 32;

const DOMAIN_SEPARATION_TAG: [u8; 23] = *b"SELECTION_RANDOMNESS_V1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection(#[serde(with = "serde_fr")] ZkHash);

pub struct ProofOfSelectionInput {
    pub secret_key: ZkHash,
    pub node_index: usize,
    pub session_number: u64,
}

impl ProofOfSelection {
    // TODO: Implement actual verification logic.
    #[must_use]
    pub fn verify(&self) -> bool {
        self == &Self::dummy()
    }

    // TODO: Remove this once the actual proof of selection is implemented.
    #[must_use]
    pub fn dummy() -> Self {
        Self(ZkHash::default())
    }

    pub fn new(
        ProofOfSelectionInput {
            node_index,
            secret_key,
            session_number,
        }: ProofOfSelectionInput,
    ) -> Self {
        let domain_separation_tag: Fr = BigUint::from_bytes_le(&DOMAIN_SEPARATION_TAG[..]).into();
        let node_index: Fr = BigUint::from_bytes_le(&node_index.to_le_bytes()[..]).into();
        let session_number: Fr = BigUint::from_bytes_le(&session_number.to_le_bytes()[..]).into();

        let hash = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[
                domain_separation_tag,
                secret_key,
                node_index,
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
