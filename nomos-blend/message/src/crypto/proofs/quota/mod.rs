use std::{ops::Deref, sync::LazyLock};

use groth16::fr_from_bytes;
use nomos_core::{blend::PROOF_OF_QUOTA_SIZE, crypto::ZkHash};
use poq::{PoQVerifierInput, PoQWitnessInputs, ProveError, prove, verify};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::crypto::proofs::{
    ZkHashExt as _,
    quota::inputs::{
        VerifyInputs,
        prove::{Inputs, PrivateInputs, PublicInputs},
    },
};

pub mod inputs;
#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "unsafe-test-functions"))]
pub mod fixtures;

/// A Proof of Quota as described in the Blend v1 spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#26a261aa09df80f4b119f900fbb36f3f>.
// TODO: To avoid proofs being misused, remove the `Clone` and `Copy` derives, so once a proof is
// verified it cannot be (mis)used anymore.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProofOfQuota(nomos_core::blend::ProofOfQuota);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid input: {0}.")]
    InvalidInput(#[from] Box<dyn core::error::Error + Send + Sync>),
    #[error("Proof generation failed: {0}.")]
    ProofGeneration(#[from] ProveError),
    #[error("Invalid proof")]
    InvalidProof,
}

impl Deref for ProofOfQuota {
    type Target = nomos_core::blend::ProofOfQuota;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<ProofOfQuota> for nomos_core::blend::ProofOfQuota {
    fn from(proof: ProofOfQuota) -> Self {
        proof.0
    }
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
            prove(witness_inputs).map_err(Error::ProofGeneration)?;
        let secret_selection_randomness = generate_secret_selection_randomness(
            secret_selection_randomness_sk,
            key_index,
            public_inputs.session,
        );
        Ok((
            Self(nomos_core::blend::ProofOfQuota {
                key_nullifier: key_nullifier.into_inner(),
                proof,
            }),
            secret_selection_randomness,
        ))
    }

    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        Self(nomos_core::blend::ProofOfQuota::from_bytes_unchecked(bytes))
    }

    /// Verify a Proof of Quota with the provided inputs.
    ///
    /// The key nullifier required to verify the proof is taken from the proof
    /// itself and is not contained in the passed inputs.
    pub fn verify(self, public_inputs: &PublicInputs) -> Result<ZkHash, Error> {
        let verifier_input =
            VerifyInputs::from_prove_inputs_and_nullifier(*public_inputs, self.key_nullifier);
        let is_proof_valid = matches!(verify(&self.proof, verifier_input.into()), Ok(true));
        if is_proof_valid {
            Ok(self.key_nullifier)
        } else {
            Err(Error::InvalidProof)
        }
    }

    #[must_use]
    pub fn key_nullifier(&self) -> ZkHash {
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
    [
        *DOMAIN_SEPARATION_TAG_FR,
        sk,
        key_index.into(),
        session.into(),
    ]
    .hash()
}
