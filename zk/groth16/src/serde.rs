pub mod serde_fr {
    use ark_bn254::Fr;
    use ark_ff::biginteger::BigInteger as _;
    use num_bigint::BigUint;
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S>(item: &Fr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex = hex::encode(item.0.to_bytes_le()); // Convert `Fr` to hex representation
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Fr, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let bytes = hex::decode(hex_str).map_err(serde::de::Error::custom)?;
        Ok(BigUint::from_bytes_le(&bytes).into()) // Parse from hex
    }
}
