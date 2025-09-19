use std::sync::LazyLock;

use ::serde::{Deserialize, Serialize};
use generic_array::{ArrayLength, GenericArray};
use groth16::{fr_from_bytes, fr_from_bytes_unchecked, Bn254, CompressSize};
use nomos_core::crypto::{ZkHash, ZkHasher};
use poq::{prove, verify, PoQProof, PoQVerifierInput, PoQWitnessInputs, ProveError};
use thiserror::Error;

use crate::crypto::proofs::quota::inputs::{
    prove::{Inputs, PrivateInputs, PublicInputs},
    VerifyInputs,
};

pub mod inputs;
mod serde;
#[cfg(test)]
mod tests;

const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
const PROOF_CIRCUIT_SIZE: usize = size_of::<PoQProof>();
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

/// A Proof of Quota as described in the Blend v1 spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#26a261aa09df80f4b119f900fbb36f3f>.
// TODO: To avoid proofs being misused, remove the `Clone` and `Copy` derives, so once a proof is
// verified it cannot be (mis)used anymore.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct ProofOfQuota {
    #[serde(with = "groth16::serde::serde_fr")]
    key_nullifier: ZkHash,
    #[serde(with = "self::serde::proof::SerializablePoQProof")]
    proof: PoQProof,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid input: {0}.")]
    InvalidInput(#[from] Box<dyn core::error::Error>),
    #[error("Proof generation failed: {0}.")]
    ProofGeneration(#[from] ProveError),
    #[error("Invalid proof")]
    InvalidProof,
}

impl ProofOfQuota {
    /// Generate a new Proof of Quota with the provided public and private
    /// inputs, along with the secret selection randomness for the Proof of
    /// Selection associated to this Proof of Quota.
    pub fn new(
        public_inputs: &PublicInputs,
        private_inputs: PrivateInputs,
    ) -> Result<(Self, ZkHash), Error> {
        let key_index = private_inputs.key_index;
        let secret_selection_randomness_sk = private_inputs.get_secret_selection_randomness_sk();
        let witness_inputs: PoQWitnessInputs = Inputs {
            private: private_inputs,
            public: *public_inputs,
        }
        .try_into()
        .map_err(|e| Error::InvalidInput(Box::new(e)))?;
        let (proof, PoQVerifierInput { key_nullifier, .. }) =
            prove(&witness_inputs).map_err(Error::ProofGeneration)?;
        let secret_selection_randomness = generate_secret_selection_randomness(
            secret_selection_randomness_sk,
            key_index,
            public_inputs.session,
        );
        Ok((
            Self {
                key_nullifier: key_nullifier.into_inner(),
                proof,
            },
            secret_selection_randomness,
        ))
    }

    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        let (key_nullifier_bytes, proof_circuit_bytes) = bytes.split_at(KEY_NULLIFIER_SIZE);
        let key_nullifier = fr_from_bytes_unchecked(key_nullifier_bytes);
        let (pi_a, pi_b, pi_c) = split_proof_components::<
            <Bn254 as CompressSize>::G1CompressedSize,
            <Bn254 as CompressSize>::G2CompressedSize,
        >(proof_circuit_bytes.try_into().unwrap());

        Self {
            key_nullifier,
            proof: PoQProof::new(pi_a, pi_b, pi_c),
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

    #[must_use]
    pub const fn key_nullifier(&self) -> ZkHash {
        self.key_nullifier
    }

    #[cfg(test)]
    #[must_use]
    pub fn dummy() -> Self {
        Self::from_bytes_unchecked([0u8; _])
    }
}

const DOMAIN_SEPARATION_TAG: [u8; 23] = *b"SELECTION_RANDOMNESS_V1";
static DOMAIN_SEPARATION_TAG_FR: LazyLock<ZkHash> = LazyLock::new(|| {
    fr_from_bytes(&DOMAIN_SEPARATION_TAG[..])
        .expect("DST for secret selection randomness calculation must be correct.")
});
// As per Proof of Quota v1 spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#215261aa09df81adb8ccd1448c9afd68>.
fn generate_secret_selection_randomness(sk: ZkHash, key_index: u64, session: u64) -> ZkHash {
    let hash_input = [
        *DOMAIN_SEPARATION_TAG_FR,
        sk,
        key_index.into(),
        session.into(),
    ];

    let mut hasher = ZkHasher::new();
    hasher.update(&hash_input);
    hasher.finalize()
}

fn split_proof_components<G1Compressed, G2Compressed>(
    bytes: [u8; PROOF_CIRCUIT_SIZE],
) -> (
    GenericArray<u8, G1Compressed>,
    GenericArray<u8, G2Compressed>,
    GenericArray<u8, G1Compressed>,
)
where
    G1Compressed: ArrayLength,
    G2Compressed: ArrayLength,
{
    let first_point_end_index = G1Compressed::USIZE;
    let second_point_end_index = first_point_end_index
        .checked_add(G2Compressed::USIZE)
        .expect("Second index overflow");
    let third_point_end_index = second_point_end_index
        .checked_add(G1Compressed::USIZE)
        .expect("Third index overflow");

    (
        GenericArray::try_from_iter(
            bytes
                .get(..first_point_end_index)
                .expect("Input byte array is not large enough for the first G1 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
        GenericArray::try_from_iter(
            bytes
                .get(first_point_end_index..second_point_end_index)
                .expect("Input byte array is not large enough for the first G2 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
        GenericArray::try_from_iter(
            bytes
                .get(second_point_end_index..third_point_end_index)
                .expect("Input byte array is not large enough for the second G1 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
    )
}
