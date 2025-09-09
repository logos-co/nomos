use ::serde::{Deserialize, Serialize};
use groth16::Fr;
use nomos_core::crypto::ZkHash;
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
const PROOF_CIRCUIT_SIZE: usize = 128;
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct ProofOfQuota {
    #[serde(with = "self::serde::input_serde")]
    key_nullifier: Fr,
    #[serde(with = "self::serde::proof_serde")]
    proof: PoQProof,
}

#[derive(Debug)]
pub enum Error {
    InvalidInput(PoQInputsFromDataError),
    ProofGeneration(ProveError),
}

impl ProofOfQuota {
    pub fn new(public_inputs: PublicInputs, private_inputs: PrivateInputs) -> Result<Self, Error> {
        let witness_inputs: PoQWitnessInputs = Inputs {
            private: private_inputs,
            public: public_inputs,
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

    pub fn verify(self, public_inputs: PublicInputs) -> Result<ZkHash, ()> {
        let verifier_input =
            VerifyInputs::from_prove_inputs_and_nullifier(public_inputs, self.key_nullifier);
        let is_proof_valid = matches!(verify(&self.proof, &verifier_input.into()), Ok(true));
        #[cfg(test)]
        // TODO: Remove this later on.
        let is_proof_valid = is_proof_valid && self == Self::dummy();
        if is_proof_valid {
            Ok(self.key_nullifier)
        } else {
            Err(())
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn dummy() -> Self {
        Self::new(PublicInputs::default(), PrivateInputs::default()).unwrap()
    }
}

impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(_value: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        todo!("Allow for random proof initialization in Blend messages.")
    }
}
