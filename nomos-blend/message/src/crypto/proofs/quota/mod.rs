use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::crypto::proofs::quota::inputs::{PrivateInputs, PublicInputs};

pub mod inputs;

const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
const PROOF_CIRCUIT_SIZE: usize = 128;
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfQuota {
    #[serde(with = "groth16::serde::serde_fr")]
    key_nullifier: ZkHash,
    #[serde(with = "serde_big_array::BigArray")]
    proof: [u8; PROOF_CIRCUIT_SIZE],
}

impl ProofOfQuota {
    #[must_use]
    pub fn new(_public_inputs: PublicInputs, _private_inputs: PrivateInputs) -> Self {
        // TODO: Interact with circom circuit generation.
        Self::dummy()
    }

    #[must_use]
    pub fn verify(self, _public_inputs: &PublicInputs) -> Result<ZkHash, ()> {
        // TODO: Interact with circom circuit verification.
        if self == Self::dummy() {
            Ok(self.key_nullifier)
        } else {
            Err(())
        }
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

impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(value: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        Self {
            key_nullifier: BigUint::from_bytes_be(&value[..KEY_NULLIFIER_SIZE]).into(),
            proof: <[u8; PROOF_CIRCUIT_SIZE]>::try_from(&value[KEY_NULLIFIER_SIZE..]).unwrap(),
        }
    }
}
