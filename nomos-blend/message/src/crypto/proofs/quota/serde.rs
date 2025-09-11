pub(super) mod input {
    use ark_ff::{biginteger::BigInteger as _, PrimeField as _};
    use nomos_core::crypto::ZkHash;
    use num_bigint::BigUint;
    use serde::{ser::Error, Deserialize as _, Deserializer, Serialize as _, Serializer};

    use crate::crypto::proofs::quota::KEY_NULLIFIER_SIZE;

    pub fn serialize<S>(input: &ZkHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let key_nullifier_bytes = <[u8; KEY_NULLIFIER_SIZE]>::try_from(
            input.into_bigint().to_bytes_le(),
        )
        .map_err(|e| S::Error::custom(format!("Key nullifier serialization error: {e:?}.")))?;

        key_nullifier_bytes.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ZkHash, D::Error>
    where
        D: Deserializer<'de>,
    {
        <[u8; 32]>::deserialize(deserializer).map(|bytes| BigUint::from_bytes_le(&bytes[..]).into())
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

    use crate::crypto::{proofs::quota::ProofOfQuota, random_sized_bytes};

    #[test]
    fn serialize_deserialize() {
        let proof = ProofOfQuota::from(random_sized_bytes());

        let serialized_proof = wire::serialize(&proof).unwrap();
        let deserialized_proof: ProofOfQuota = wire::deserialize(&serialized_proof[..]).unwrap();

        assert!(proof == deserialized_proof);
    }
}
