use ::serde::{Deserialize, Serialize};
use groth16::{BN254_G1_COMPRESSED_SIZE, BN254_G2_COMPRESSED_SIZE};
use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;
use poq::{
    prove, verify, PoQInputsFromDataError, PoQProof, PoQVerifierInput, PoQWitnessInputs, ProveError,
};

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

#[derive(Debug)]
pub enum Error {
    InvalidInput(PoQInputsFromDataError),
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
        .map_err(Error::InvalidInput)?;
        let (proof, PoQVerifierInput { key_nullifier, .. }) =
            prove(&witness_inputs).map_err(Error::ProofGeneration)?;
        Ok(Self {
            key_nullifier: key_nullifier.into_inner(),
            proof,
        })
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
}

#[expect(
    clippy::fallible_impl_from,
    reason = "We have a fixed-size input, so this will never actually panic."
)]
impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(value: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        const FIRST_POINT_END_INDEX: usize = BN254_G1_COMPRESSED_SIZE;
        const SECOND_POINT_END_INDEX: usize = FIRST_POINT_END_INDEX
            .checked_add(BN254_G2_COMPRESSED_SIZE)
            .expect("Index overflow");
        const THIRD_POINT_END_INDEX: usize = SECOND_POINT_END_INDEX
            .checked_add(BN254_G1_COMPRESSED_SIZE)
            .expect("Index overflow");

        let (key_nullifier, compressed_proof) = value.split_at(KEY_NULLIFIER_SIZE);

        Self {
            key_nullifier: BigUint::from_bytes_be(key_nullifier).into(),
            proof: PoQProof::from_components(
                compressed_proof
                    .get(..FIRST_POINT_END_INDEX)
                    .unwrap()
                    .try_into()
                    .unwrap(),
                compressed_proof
                    .get(FIRST_POINT_END_INDEX..SECOND_POINT_END_INDEX)
                    .unwrap()
                    .try_into()
                    .unwrap(),
                compressed_proof
                    .get(SECOND_POINT_END_INDEX..THIRD_POINT_END_INDEX)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
        }
    }
}
