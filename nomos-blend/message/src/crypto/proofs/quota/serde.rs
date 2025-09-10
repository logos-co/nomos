pub(super) mod input {
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

pub(super) mod proof {
    use nomos_utils::serde::deserialize_bytes_array;
    use poq::PoQProof;
    use serde::{de::Error as _, Deserializer, Serialize, Serializer};

    use crate::crypto::proofs::quota::{KEY_NULLIFIER_SIZE, PROOF_OF_QUOTA_SIZE};

    pub fn serialize<S>(proof: &PoQProof, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        [&proof.pi_a[..], &proof.pi_b[..], &proof.pi_c[..]].serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PoQProof, D::Error>
    where
        D: Deserializer<'de>,
    {
        let concatenated = deserialize_bytes_array::<PROOF_OF_QUOTA_SIZE, D>(deserializer)?;
        // TODO: Resume from here
    }
}
