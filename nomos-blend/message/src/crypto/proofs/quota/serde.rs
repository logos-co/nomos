pub(super) mod input {
    use ark_ff::biginteger::BigInteger as _;
    use nomos_core::crypto::ZkHash;
    use num_bigint::BigUint;
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    pub fn serialize<S>(input: &ZkHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        input.0.to_bytes_le().serialize(serializer)
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
        let serialization_helper = SerializationHelper::deserialize(deserializer)?;
        Ok(serialization_helper.into())
    }
}
