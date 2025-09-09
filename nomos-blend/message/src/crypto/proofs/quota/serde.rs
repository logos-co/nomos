use groth16::Fr;
use poq::PoQProof;
use serde::{Deserialize, Serialize};

use crate::crypto::proofs::quota::ProofOfQuota;

#[derive(Serialize, Deserialize)]
struct ProofOfQuotaSerde {
    #[serde(with = "input_serde")]
    key_nullifier: Fr,
    #[serde(with = "proof_serde")]
    proof: PoQProof,
}

impl From<ProofOfQuotaSerde> for ProofOfQuota {
    fn from(value: ProofOfQuotaSerde) -> Self {
        Self {
            key_nullifier: value.key_nullifier,
            proof: value.proof,
        }
    }
}

pub(super) mod input_serde {
    use groth16::Fr;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(_input: &Fr, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        todo!("Implement proper serialization later.")
    }

    pub fn deserialize<'de, D>(_deserializer: D) -> Result<Fr, D::Error>
    where
        D: Deserializer<'de>,
    {
        todo!("Implement proper serialization later.")
    }
}

// TODO: Implement proper serialization later.
pub(super) mod proof_serde {
    use poq::PoQProof;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(_proof: &PoQProof, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        todo!("Implement proper serialization later.")
    }

    pub fn deserialize<'de, D>(_deserializer: D) -> Result<PoQProof, D::Error>
    where
        D: Deserializer<'de>,
    {
        todo!("Implement proper serialization later.")
    }
}
