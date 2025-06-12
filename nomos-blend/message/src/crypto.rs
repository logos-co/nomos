use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::serde::FixedSizeSerde;

const PUBLIC_KEY_SIZE: usize = 32;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct PublicKey(pub [u8; PUBLIC_KEY_SIZE]);

impl FixedSizeSerde for PublicKey {
    const SIZE: usize = size_of::<u8>() * PUBLIC_KEY_SIZE;
}

const PROOF_OF_QUOTA_SIZE: usize = 160;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct ProofOfQuota(#[serde(with = "BigArray")] pub [u8; PROOF_OF_QUOTA_SIZE]);

impl FixedSizeSerde for ProofOfQuota {
    const SIZE: usize = size_of::<u8>() * PROOF_OF_QUOTA_SIZE;
}

const PROOF_OF_SELECTION_SIZE: usize = 32;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct ProofOfSelection(pub [u8; PROOF_OF_SELECTION_SIZE]);

impl FixedSizeSerde for ProofOfSelection {
    const SIZE: usize = size_of::<u8>() * PROOF_OF_SELECTION_SIZE;
}

const SIGNATURE_SIZE: usize = 64;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct Signature(#[serde(with = "BigArray")] pub [u8; SIGNATURE_SIZE]);

impl FixedSizeSerde for Signature {
    const SIZE: usize = size_of::<u8>() * SIGNATURE_SIZE;
}
