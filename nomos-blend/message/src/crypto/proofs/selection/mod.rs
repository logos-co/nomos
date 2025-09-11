use ::serde::{Deserialize, Serialize};
use groth16::{fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use nomos_core::crypto::{ZkHash, ZkHasher};
use num_bigint::BigUint;

use crate::crypto::{
    blake2b512, proofs::selection::inputs::VerifyInputs, pseudo_random_sized_bytes,
};

pub mod inputs;
mod serde;

pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ProofOfSelection>();
const DOMAIN_SEPARATION_TAG: [u8; 9] = *b"BlendNode";

/// A Proof of Selection as described in the Blend v1 spec: <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81d6bb3febd62b598138>.
// TODO: To avoid proofs being misused, remove the `Clone` and `Copy` derives,
// so once a proof is verified it cannot be (mis)used anymore.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection {
    #[serde(with = "self::serde::selection_randomness")]
    selection_randomness: ZkHash,
}

pub enum Error {
    IndexMismatch { expected: u64, provided: u64 },
    Overflow,
    KeyNullifierMismatch { expected: ZkHash, provided: ZkHash },
    InvalidInput(Box<dyn core::error::Error>),
}

impl ProofOfSelection {
    #[must_use]
    pub const fn new(selection_randomness: ZkHash) -> Self {
        Self {
            selection_randomness,
        }
    }

    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_SELECTION_SIZE]) -> Self {
        Self {
            selection_randomness: fr_from_bytes_unchecked(&bytes),
        }
    }

    pub(super) fn verify(
        self,
        VerifyInputs {
            expected_node_index,
            key_nullifier,
            total_membership_size,
        }: &VerifyInputs,
    ) -> Result<(), Error> {
        // Condition 1: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df819991e6f9455ff7ec92
        let selection_randomness_bytes = fr_to_bytes(&self.selection_randomness);
        let selection_randomness_blake_hash =
            blake2b512(&[&DOMAIN_SEPARATION_TAG[..], &selection_randomness_bytes[..]].concat());
        let pseudo_random_output: u64 = {
            let pseudo_random_output_bytes =
                pseudo_random_sized_bytes::<8>(&selection_randomness_blake_hash);
            let pseudo_random_biguint = BigUint::from_bytes_be(&pseudo_random_output_bytes[..]);
            pseudo_random_biguint
                .try_into()
                .map_err(|_| Error::Overflow)?
        };
        let final_index = pseudo_random_output % total_membership_size;
        if final_index != *expected_node_index {
            return Err(Error::IndexMismatch {
                expected: *expected_node_index,
                provided: final_index,
            });
        }

        // Condition 2: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df814da8e8ec1f1fcf4fe6
        let selection_randomness_zk_hash = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[self.selection_randomness]);
            hasher.finalize()
        };
        if selection_randomness_zk_hash != *key_nullifier {
            return Err(Error::KeyNullifierMismatch {
                expected: *key_nullifier,
                provided: selection_randomness_zk_hash,
            });
        }

        Ok(())
    }

    #[cfg(test)]
    #[must_use]
    pub fn dummy() -> Self {
        Self::from_bytes_unchecked([0u8; _])
    }
}

impl TryFrom<[u8; PROOF_OF_SELECTION_SIZE]> for ProofOfSelection {
    type Error = Error;

    fn try_from(value: [u8; PROOF_OF_SELECTION_SIZE]) -> Result<Self, Self::Error> {
        Ok(Self {
            selection_randomness: fr_from_bytes(&value)
                .map_err(|e| Error::InvalidInput(Box::new(e)))?,
        })
    }
}
