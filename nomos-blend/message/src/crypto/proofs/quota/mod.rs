use nomos_core::crypto::ZkHash;
use serde::{Deserialize, Serialize};

use crate::crypto::proofs::quota::inputs::{PrivateInputs, PublicInputs};

pub mod inputs;

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfQuota {
    #[serde(with = "groth16::serde::serde_fr")]
    key_nullifier: ZkHash,
    #[serde(with = "serde_big_array::BigArray")]
    proof: [u8; 128],
}

impl ProofOfQuota {
    #[must_use]
    pub fn new(_public_inputs: PublicInputs, _private_inputs: PrivateInputs) -> Self {
        // TODO: Interact with circom circuit generation.
        Self::dummy()
    }

    #[must_use]
    pub fn verify(self, _public_inputs: &PublicInputs) -> bool {
        // TODO: Interact with circom circuit verification.
        self == Self::dummy()
    }

    // TODO: Remove this once the actual proof of quota is implemented.
    #[must_use]
    pub fn dummy() -> Self {
        Self {
            key_nullifier: ZkHash::default(),
            proof: [0u8; _],
        }
    }
}
