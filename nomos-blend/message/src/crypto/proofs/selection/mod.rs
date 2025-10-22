use core::fmt::Debug;
use std::sync::LazyLock;

use ::serde::{Deserialize, Serialize};
use groth16::{fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;
use thiserror::Error;

use crate::crypto::{
    blake2b512,
    proofs::{ZkHashExt as _, selection::inputs::VerifyInputs},
    pseudo_random_sized_bytes,
};

pub mod inputs;

#[cfg(test)]
mod tests;

pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ProofOfSelection>();
const DOMAIN_SEPARATION_TAG: [u8; 9] = *b"BlendNode";

/// A Proof of Selection as described in the Blend v1 spec: <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81d6bb3febd62b598138>.
// TODO: To avoid proofs being misused, remove the `Clone` and `Copy` derives,
// so once a proof is verified it cannot be (mis)used anymore.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProofOfSelection {
    #[serde(with = "groth16::serde::serde_fr")]
    selection_randomness: ZkHash,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Index mismatch. Expected {expected}, provided {provided}.")]
    IndexMismatch { expected: u64, provided: u64 },
    #[error("Overflow when verifying PoSel.")]
    Overflow,
    #[error("Key nullifier mismatch. Expected {expected}, provided {provided}.")]
    KeyNullifierMismatch { expected: ZkHash, provided: ZkHash },
    #[error("Invalid input: {0}.")]
    InvalidInput(Box<dyn core::error::Error>),
    #[error("Proof of Selection verification failed.")]
    Verification,
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

    /// Returns the index the Proof of Selection refers to, for the provided
    /// membership size.
    pub fn expected_index(&self, membership_size: usize) -> Result<usize, Error> {
        // Condition 1: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df819991e6f9455ff7ec92
        let selection_randomness_bytes = fr_to_bytes(&self.selection_randomness);
        let selection_randomness_blake_hash =
            blake2b512(&[&DOMAIN_SEPARATION_TAG[..], &selection_randomness_bytes[..]]);
        let pseudo_random_output: u64 = {
            let pseudo_random_output_bytes =
                pseudo_random_sized_bytes::<8>(&selection_randomness_blake_hash);
            let pseudo_random_biguint = BigUint::from_bytes_le(&pseudo_random_output_bytes[..]);
            pseudo_random_biguint
                .try_into()
                .map_err(|_| Error::Overflow)?
        };
        (pseudo_random_output % u64::try_from(membership_size).map_err(|_| Error::Overflow)?)
            .try_into()
            .map_err(|_| Error::Overflow)
    }

    pub fn verify(
        self,
        VerifyInputs {
            expected_node_index,
            key_nullifier,
            total_membership_size,
        }: &VerifyInputs,
    ) -> Result<(), Error> {
        let final_index = self.expected_index(*total_membership_size as usize)?;
        if final_index != *expected_node_index as usize {
            return Err(Error::IndexMismatch {
                expected: final_index as u64,
                provided: *expected_node_index,
            });
        }

        // Condition 2: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df814da8e8ec1f1fcf4fe6
        let calculated_key_nullifier =
            derive_key_nullifier_from_secret_selection_randomness(self.selection_randomness);
        if calculated_key_nullifier != *key_nullifier {
            return Err(Error::KeyNullifierMismatch {
                expected: calculated_key_nullifier,
                provided: *key_nullifier,
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

const KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG: [u8; 16] = *b"KEY_NULLIFIER_V1";
static KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG_FR: LazyLock<ZkHash> = LazyLock::new(|| {
    fr_from_bytes(&KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG[..]).expect(
        "DST for key nullifier derivation from secret selection randomness must be correct.",
    )
});
// As per Proof of Quota v1 spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81adb8ccd1448c9afd68>.
#[must_use]
pub fn derive_key_nullifier_from_secret_selection_randomness(
    secret_selection_randomness: ZkHash,
) -> ZkHash {
    [
        *KEY_NULLIFIER_DERIVATION_DOMAIN_SEPARATION_TAG_FR,
        secret_selection_randomness,
    ]
    .hash()
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
