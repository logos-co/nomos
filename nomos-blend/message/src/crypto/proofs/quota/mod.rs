use ::serde::{Deserialize, Serialize};
use groth16::{Fr, Groth16Input};
use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;
use poq::{prove, verify, PoQInputsFromDataError, PoQProof, PoQWitnessInputs, ProveError};

use crate::crypto::proofs::quota::inputs::{
    prove::{Inputs, PrivateInputs, PublicInputs},
    verify::Inputs as VerifyInputs,
};

pub mod inputs;
mod serde;

const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
const PROOF_CIRCUIT_SIZE: usize = 128;
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct ProofOfQuota {
    #[serde(with = "self::serde::input_serde")]
    key_nullifier: Fr,
    #[serde(with = "self::serde::proof_serde")]
    proof: PoQProof,
}

pub enum Error {
    InvalidInput(PoQInputsFromDataError),
    ProofGeneration(ProveError),
}

impl ProofOfQuota {
    #[must_use]
    pub fn new(public_inputs: PublicInputs, private_inputs: PrivateInputs) -> Result<Self, Error> {
        let witness_inputs: PoQWitnessInputs = Inputs {
            private: private_inputs,
            public: public_inputs,
        }
        .try_into()
        .map_err(Error::InvalidInput)?;
        let (proof, verifier_input) = prove(&witness_inputs).map_err(Error::ProofGeneration)?;
        let key_nullifier = verifier_input.key_nullifier;
        Ok(Self {
            key_nullifier: key_nullifier.into_inner(),
            proof,
        })
    }

    #[must_use]
    pub fn verify(self, public_inputs: VerifyInputs) -> Result<ZkHash, ()> {
        let is_proof_valid = matches!(verify(&self.proof, &public_inputs.into()), Ok(true));
        if is_proof_valid {
            Ok(self.key_nullifier)
        } else {
            Err(())
        }
    }
}

impl From<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    fn from(_value: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        todo!("Allow for random proof initialization in Blend messages.")
    }
}
