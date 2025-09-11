use ::serde::{Deserialize, Serialize};
use groth16::{
    fr_from_bytes, fr_from_bytes_unchecked, BN254_G1_COMPRESSED_SIZE, BN254_G2_COMPRESSED_SIZE,
};
use nomos_core::crypto::ZkHash;
use poq::{prove, verify, PoQProof, PoQVerifierInput, PoQWitnessInputs, ProveError};

use crate::crypto::proofs::quota::inputs::{
    prove::{Inputs, PrivateInputs, PublicInputs},
    VerifyInputs,
};

pub mod inputs;
mod serde;

const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
const PROOF_CIRCUIT_SIZE: usize = size_of::<PoQProof>();
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

/// A Proof of Quota as described in the Blend v1 spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#26a261aa09df80f4b119f900fbb36f3f>.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct ProofOfQuota {
    #[serde(with = "self::serde::key_nullifier")]
    key_nullifier: ZkHash,
    #[serde(with = "self::serde::proof")]
    proof: PoQProof,
}

pub enum Error {
    InvalidInput(Box<dyn core::error::Error>),
    ProofGeneration(ProveError),
    InvalidProof,
}

impl ProofOfQuota {
    /// Generate a new Proof of Quota with the provided public and private
    /// inputs.
    pub fn new(public_inputs: &PublicInputs, private_inputs: PrivateInputs) -> Result<Self, Error> {
        let witness_inputs: PoQWitnessInputs = Inputs {
            private: private_inputs,
            public: *public_inputs,
        }
        .try_into()
        .map_err(|e| Error::InvalidInput(Box::new(e)))?;
        let (proof, PoQVerifierInput { key_nullifier, .. }) =
            prove(&witness_inputs).map_err(Error::ProofGeneration)?;
        Ok(Self {
            key_nullifier: key_nullifier.into_inner(),
            proof,
        })
    }

    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        let (key_nullifier_bytes, proof_circuit_bytes) = bytes.split_at(KEY_NULLIFIER_SIZE);
        let key_nullifier = fr_from_bytes_unchecked(key_nullifier_bytes.try_into().unwrap());
        let (pi_a, pi_b, pi_c) = split_proof_components(proof_circuit_bytes.try_into().unwrap());

        Self {
            key_nullifier,
            proof: PoQProof::from_components(pi_a, pi_b, pi_c),
        }
    }

    /// Verify a Proof of Quota with the provided inputs.
    ///
    /// The key nullifier required to verify the proof is taken from the proof
    /// itself and is not contained in the passed inputs.
    pub(super) fn verify(self, public_inputs: &PublicInputs) -> Result<ZkHash, Error> {
        let verifier_input =
            VerifyInputs::from_prove_inputs_and_nullifier(*public_inputs, self.key_nullifier);
        let is_proof_valid = matches!(verify(&self.proof, &verifier_input.into()), Ok(true));
        if is_proof_valid {
            Ok(self.key_nullifier)
        } else {
            Err(Error::InvalidProof)
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn dummy() -> Self {
        Self::from_bytes_unchecked([0u8; _])
    }
}

fn split_proof_components(
    bytes: [u8; PROOF_CIRCUIT_SIZE],
) -> (
    [u8; BN254_G1_COMPRESSED_SIZE],
    [u8; BN254_G2_COMPRESSED_SIZE],
    [u8; BN254_G1_COMPRESSED_SIZE],
) {
    const FIRST_POINT_END_INDEX: usize = BN254_G1_COMPRESSED_SIZE;
    const SECOND_POINT_END_INDEX: usize = FIRST_POINT_END_INDEX
        .checked_add(BN254_G2_COMPRESSED_SIZE)
        .expect("Second index overflow");
    const THIRD_POINT_END_INDEX: usize = SECOND_POINT_END_INDEX
        .checked_add(BN254_G1_COMPRESSED_SIZE)
        .expect("Third index overflow");

    (
        bytes
            .get(..FIRST_POINT_END_INDEX)
            .unwrap()
            .try_into()
            .unwrap(),
        bytes
            .get(FIRST_POINT_END_INDEX..SECOND_POINT_END_INDEX)
            .unwrap()
            .try_into()
            .unwrap(),
        bytes
            .get(SECOND_POINT_END_INDEX..THIRD_POINT_END_INDEX)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

impl TryFrom<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    type Error = Error;

    fn try_from(value: [u8; PROOF_OF_QUOTA_SIZE]) -> Result<Self, Self::Error> {
        let (key_nullifier_bytes, proof_circuit_bytes) = value.split_at(KEY_NULLIFIER_SIZE);

        let key_nullifier = fr_from_bytes(key_nullifier_bytes.try_into().unwrap())
            .map_err(|e| Error::InvalidInput(Box::new(e)))?;
        let (pi_a, pi_b, pi_c) = split_proof_components(proof_circuit_bytes.try_into().unwrap());

        Ok(Self {
            key_nullifier,
            proof: PoQProof::from_components(pi_a, pi_b, pi_c),
        })
    }
}
