pub(super) mod key_nullifier {
    use groth16::{fr_from_bytes, fr_to_bytes};
    use nomos_core::crypto::ZkHash;
    use serde::{de::Error, Deserialize, Deserializer, Serialize as _, Serializer};

    pub fn serialize<S>(input: &ZkHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        fr_to_bytes(*input).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ZkHash, D::Error>
    where
        D: Deserializer<'de>,
    {
        fr_from_bytes(&Deserialize::deserialize(deserializer)?).map_err(Error::custom)
    }
}

pub(super) mod proof {
    use groth16::{BN254_G1_COMPRESSED_SIZE, BN254_G2_COMPRESSED_SIZE};
    use poq::PoQProof;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct SerializationHelper(
        [u8; BN254_G1_COMPRESSED_SIZE],
        #[serde(with = "serde_big_array::BigArray")] [u8; BN254_G2_COMPRESSED_SIZE],
        [u8; BN254_G1_COMPRESSED_SIZE],
    );

    impl From<PoQProof> for SerializationHelper {
        fn from(
            PoQProof {
                pi_a, pi_b, pi_c, ..
            }: PoQProof,
        ) -> Self {
            Self(pi_a, pi_b, pi_c)
        }
    }

    impl From<SerializationHelper> for PoQProof {
        fn from(SerializationHelper(pi_a, pi_b, pi_c): SerializationHelper) -> Self {
            Self::from_components(pi_a, pi_b, pi_c)
        }
    }

    pub fn serialize<S>(proof: &PoQProof, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerializationHelper::from(*proof).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PoQProof, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(SerializationHelper::deserialize(deserializer)?.into())
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::wire;

    use crate::crypto::proofs::quota::ProofOfQuota;

    #[test]
    fn serialize_deserialize() {
        let proof = ProofOfQuota::from_bytes_unchecked([0; _]);

        let serialized_proof = wire::serialize(&proof).unwrap();
        let deserialized_proof: ProofOfQuota = wire::deserialize(&serialized_proof[..]).unwrap();

        assert!(proof == deserialized_proof);
    }
}
