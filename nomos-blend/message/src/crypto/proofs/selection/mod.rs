use ::serde::{Deserialize, Serialize};
use groth16::Fr;
use nomos_core::crypto::ZkHasher;
use num_bigint::BigUint;

use crate::crypto::{
    blake2b512, proofs::selection::inputs::VerifyInputs, pseudo_random_sized_bytes,
};

pub mod inputs;
mod serde;

pub const PROOF_OF_SELECTION_SIZE: usize = 32;
const DOMAIN_SEPARATION_TAG: [u8; 9] = *b"BlendNode";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofOfSelection {
    #[serde(with = "self::serde::selection_randomness")]
    selection_randomness: Fr,
}

#[derive(Debug)]
pub enum Error {
    IndexMismatch { expected: u64, provided: u64 },
    Overflow,
    KeyNullifierMismatch { expected: Fr, provided: Fr },
}

impl ProofOfSelection {
    #[must_use]
    pub const fn new(selection_randomness: Fr) -> Self {
        Self {
            selection_randomness,
        }
    }

    pub fn verify(
        self,
        VerifyInputs {
            expected_index,
            key_nullifier,
            total_count,
        }: VerifyInputs,
    ) -> Result<(), Error> {
        #[cfg(test)]
        if self == Self::always_valid() {
            return Ok(());
        }

        // Condition 1: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df819991e6f9455ff7ec92
        let selection_randomness_bytes = {
            let selection_randomness_bigint: BigUint = self.selection_randomness.into();
            selection_randomness_bigint.to_bytes_be()
        };
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
        let final_index = pseudo_random_output % total_count;
        if final_index != expected_index {
            return Err(Error::IndexMismatch {
                expected: expected_index,
                provided: final_index,
            });
        }

        // Condition 2: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df814da8e8ec1f1fcf4fe6
        let selection_randomness_zk_hash = {
            let mut hasher = ZkHasher::new();
            hasher.update(&[self.selection_randomness]);
            hasher.finalize()
        };
        if selection_randomness_zk_hash != key_nullifier {
            return Err(Error::KeyNullifierMismatch {
                expected: key_nullifier,
                provided: selection_randomness_zk_hash,
            });
        }

        Ok(())
    }

    #[cfg(test)]
    #[must_use]
    pub const fn always_valid() -> Self {
        use groth16::Field as _;

        Self::new(Fr::ZERO)
    }
}

impl From<[u8; PROOF_OF_SELECTION_SIZE]> for ProofOfSelection {
    fn from(_value: [u8; PROOF_OF_SELECTION_SIZE]) -> Self {
        todo!("Allow for random proof initialization in Blend messages.")
    }
}
